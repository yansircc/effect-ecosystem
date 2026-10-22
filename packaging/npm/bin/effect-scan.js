#!/usr/bin/env node
// Resolves the platform package that carries the native effect-scan binary and runs it with this
// process's arguments, stdio and exit status. No behaviour lives here.
import { spawnSync } from "node:child_process"
import { readFileSync } from "node:fs"
import { createRequire } from "node:module"
import { dirname, join } from "node:path"

const require = createRequire(import.meta.url)
const { name } = JSON.parse(readFileSync(new URL("../package.json", import.meta.url), "utf8"))
const platformPackage = `${name}-${process.platform}-${process.arch}`

let binary
try {
  binary = join(dirname(require.resolve(`${platformPackage}/package.json`)), "bin", "effect-scan")
} catch {
  console.error(`effect-scan: no binary for ${process.platform}-${process.arch} (${platformPackage} is not installed).`)
  console.error("Supported: darwin-arm64, linux-x64, linux-arm64. Reinstall without --omit=optional.")
  process.exit(2)
}

const result = spawnSync(binary, process.argv.slice(2), { stdio: "inherit" })
if (result.error) {
  console.error(`effect-scan: cannot run ${binary}: ${result.error.message}`)
  process.exit(2)
}
if (result.signal) process.kill(process.pid, result.signal)
process.exit(result.status ?? 2)
