#!/usr/bin/env node
// Resolves the platform package that carries the native effect-scan binary and runs it with this
// process's arguments, stdio and exit status. No behaviour lives here except saying precisely why a
// binary cannot be used (wrong libc, too-old glibc, Rosetta, missing platform package) and relaying
// termination signals, so killing the shim never orphans the scanner.
import { spawn, spawnSync } from "node:child_process"
import { existsSync, readFileSync } from "node:fs"
import { createRequire } from "node:module"
import { dirname, join } from "node:path"
import { compareVersions } from "./glibc.mjs"

const require = createRequire(import.meta.url)
const { name } = JSON.parse(readFileSync(new URL("../package.json", import.meta.url), "utf8"))
const platformKey = `${process.platform}-${process.arch}`
const platformPackage = `${name}-${platformKey}`
const SUPPORTED = "darwin-arm64, linux-x64 (glibc), linux-arm64 (glibc)"

const fail = (...lines) => {
  for (const line of lines) console.error(`effect-scan: ${line}`)
  process.exit(2)
}

// The C library of this Node process: glibc reports its version, musl and others do not.
const hostLibc = () => {
  if (process.platform !== "linux") return undefined
  if (process.report) process.report.excludeNetwork = true
  const version = process.report?.getReport().header.glibcVersionRuntime
  return version ? { kind: "glibc", version } : { kind: "other" }
}

// An x64 Node on an Apple Silicon Mac (Rosetta) cannot be told from an Intel Mac by process.arch.
const underRosetta = () =>
  process.platform === "darwin" &&
  process.arch === "x64" &&
  spawnSync("sysctl", ["-n", "sysctl.proc_translated"], { encoding: "utf8" }).stdout?.trim() === "1"

let packageDir
try {
  packageDir = dirname(require.resolve(`${platformPackage}/package.json`))
} catch {
  if (underRosetta()) {
    fail(
      "this Node.js is an x64 build running under Rosetta on an Apple Silicon Mac, so npm selected darwin-x64, which has no binary.",
      "Install an arm64 (native) Node.js and reinstall @yansircc/effect-scan; the darwin-arm64 binary is what runs here.",
    )
  }
  if (hostLibc()?.kind === "other") {
    fail(
      `${platformKey} without glibc (musl, e.g. Alpine) is not supported, so ${platformPackage} was not installed.`,
      `Supported: ${SUPPORTED}. Use a glibc-based image (Debian/Ubuntu), or the skill on a supported host.`,
    )
  }
  fail(
    `no binary for ${platformKey} (${platformPackage} is not installed).`,
    `Supported: ${SUPPORTED}. If this platform is supported, reinstall without --omit=optional / --no-optional.`,
  )
}

const binary = join(packageDir, "bin", "effect-scan")
const { effectScan = {} } = JSON.parse(readFileSync(join(packageDir, "package.json"), "utf8"))
const libc = hostLibc()
if (libc?.kind === "other") {
  fail(`this ${platformKey} Node.js does not run on glibc (musl?); the bundled binary needs glibc >= ${effectScan.minGlibc ?? "2"}.`, `Supported: ${SUPPORTED}.`)
}
if (libc && effectScan.minGlibc && compareVersions(libc.version, effectScan.minGlibc) < 0) {
  fail(
    `this host has glibc ${libc.version}; the bundled ${platformKey} binary needs glibc >= ${effectScan.minGlibc}.`,
    "Use a newer distribution (e.g. Ubuntu 24.04+, Debian 13+), or run the scanner in a container with a newer glibc.",
  )
}

const child = spawn(binary, process.argv.slice(2), { stdio: "inherit" })
const relay = ["SIGINT", "SIGTERM", "SIGHUP"].map((signal) => {
  const handler = () => child.kill(signal)
  process.on(signal, handler)
  return [signal, handler]
})

child.on("error", (error) => {
  const reason =
    error.code === "ENOENT" && existsSync(binary)
      ? "the binary exists but its dynamic loader is missing (a non-glibc libc, or a 32-bit userland)"
      : error.code === "ENOEXEC" || error.code === "EBADARCH"
        ? `the binary is not executable on this CPU (${platformKey} host, wrong architecture or emulation)`
        : error.code === "EACCES"
          ? "the binary is not executable (was the package extracted with noexec or without modes?)"
          : error.message
  fail(`cannot run ${binary}: ${reason}.`)
})

child.on("exit", (code, signal) => {
  // Re-raise the child's signal on ourselves with default handling so our parent sees the same death.
  if (signal) {
    for (const [name, handler] of relay) process.off(name, handler)
    process.kill(process.pid, signal)
    return
  }
  process.exit(code ?? 2)
})
