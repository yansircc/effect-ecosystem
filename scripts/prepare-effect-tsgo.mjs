import { spawnSync } from "node:child_process"
import { createRequire } from "node:module"
import { dirname, resolve } from "node:path"

const root = resolve(import.meta.dirname, "..")
const require = createRequire(import.meta.url)
const tsc = resolve(root, "node_modules/typescript/bin/tsc")
const effectTsgo = resolve(
  dirname(require.resolve("@effect/tsgo/package.json")),
  "dist/effect-tsgo.cjs",
)
// The patch writes the provider's own version into the compiler marker, so the expected string is
// read from the two installed packages instead of being written down a second time here. A marker
// that carries another version is a stale patch from an earlier provider, and is replaced too.
const expected = `+effect-tsgo.${require("@effect/tsgo/package.json").version}`

const run = (entry, args, stdio = "pipe") =>
  spawnSync(process.execPath, [entry, ...args], { cwd: root, encoding: "utf8", stdio })

const current = run(tsc, ["--version"])
if (current.status !== 0) throw current.error ?? new Error(current.stderr || "tsc --version failed")
if (!current.stdout.includes(expected)) {
  const patched = run(
    effectTsgo,
    ["patch", "--typescript", "--typescript-package", "typescript"],
    "inherit",
  )
  if (patched.status !== 0) {
    throw patched.error ?? new Error(`effect-tsgo patch exited ${String(patched.status)}`)
  }
}
