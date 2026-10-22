#!/usr/bin/env node
// Self-test of the release machinery that CI cannot exercise before it matters: bump resolution,
// release identity in new/existing/queued/stale situations (real git repos), the post-publish record
// step under a rejected push, glibc detection, and the npm shim's exit/signal behaviour.
// No network. Run by verify-package.mjs (make verify-offline).
import assert from "node:assert/strict"
import { execFileSync, spawn, spawnSync } from "node:child_process"
import { chmodSync, copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { join, resolve } from "node:path"
import { compareVersions, maxGlibcVersion } from "../../packaging/npm/bin/glibc.mjs"
import { bumpVersion, releaseBumpFromMessage, releaseBumpFromMessages } from "./version.mjs"

const here = import.meta.dirname
const repoRoot = resolve(here, "../..")
const temporary = mkdtempSync(join(tmpdir(), "effect-scan-release-selftest-"))
const node = process.execPath

try {
  // --- bump resolution -------------------------------------------------------------------------
  assert.equal(releaseBumpFromMessage("fix: x"), "patch")
  assert.equal(releaseBumpFromMessage("feat\n\nRelease-Bump: minor\n"), "minor")
  // squash body repeating one value, or mixing values: tolerated, greatest wins
  assert.equal(releaseBumpFromMessage("a\n\nRelease-Bump: minor\nRelease-Bump: minor\n"), "minor")
  const mixed = releaseBumpFromMessages(["a\n\nRelease-Bump: patch", "b\n\nRelease-Bump: major", "c"])
  assert.deepEqual(mixed, { bump: "major", declared: ["patch", "major"], conflicts: true })
  // a release commit's own trailer is not input
  assert.equal(releaseBumpFromMessages(["chore(release)\n\nRelease-Source: abc1234\nRelease-Bump: major"]).bump, "patch")
  assert.throws(() => releaseBumpFromMessage("x\n\nRelease-Bump: huge"), /major, minor, or patch/)
  assert.equal(bumpVersion("0.1.9", "minor"), "0.2.0")

  // --- glibc ------------------------------------------------------------------------------------
  const objdump = "0000 DF *UND* 0000 (GLIBC_2.17) memcpy\n0000 DF *UND* 0000 (GLIBC_2.39) pidfd_spawn\n0000 DF *UND* 0000 GLIBC_2.4 x"
  assert.equal(maxGlibcVersion(objdump), "2.39")
  assert.equal(maxGlibcVersion("no versions"), undefined)
  assert.ok(compareVersions("2.9", "2.39") < 0 && compareVersions("2.39", "2.39") === 0 && compareVersions("2.40", "2.39") > 0)

  // --- release identity and record step, in real repositories ----------------------------------
  const git = (cwd, ...args) => execFileSync("git", args, { cwd, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }).trim()
  const origin = join(temporary, "origin.git")
  const work = join(temporary, "work")
  git(temporary, "init", "-q", "--bare", "-b", "main", origin)
  git(temporary, "clone", "-q", origin, work)
  git(work, "config", "user.name", "t")
  git(work, "config", "user.email", "t@example.invalid")
  git(work, "checkout", "-q", "-b", "main")
  const writeJson = (file, value) => writeFileSync(join(work, file), `${JSON.stringify(value, null, 2)}\n`)
  const commit = (message, file = "a.txt") => {
    writeFileSync(join(work, file), `${message}\n${Math.random()}\n`)
    git(work, "add", "-A")
    git(work, "commit", "-q", "-m", message)
    return git(work, "rev-parse", "HEAD")
  }
  writeJson("package.json", { name: "@x/y", version: "0.1.0" })
  writeJson("package-lock.json", { name: "@x/y", version: "0.1.0", packages: { "": { version: "0.1.0" } } })
  const c0 = commit("initial")
  git(work, "push", "-q", "origin", "main")

  const prepare = (sha) => {
    git(work, "checkout", "-q", "--force", "--detach", "origin/main")
    const out = execFileSync(node, [join(here, "prepare.mjs"), sha, "--root", work], { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] })
    return JSON.parse(out)
  }
  const record = (identity, env = {}) =>
    spawnSync(
      node,
      [
        join(here, "push-release.mjs"),
        "--root", work, "--version", identity.version, "--tag", identity.tag, "--bump", identity.bump,
        "--source", identity.sourceSha, "--files", identity.releaseFiles,
      ],
      { encoding: "utf8", env: { ...process.env, PUSH_RELEASE_RETRY_MS: "10", ...env } },
    )
  const reset = () => git(work, "checkout", "-q", "--force", "main")

  // no previous release: head only, patch
  const first = prepare(c0)
  assert.deepEqual([first.mode, first.version, first.bump, first.releaseFiles], ["new", "0.1.1", "patch", "package.json,package-lock.json"])
  assert.equal(git(work, "rev-parse", "HEAD"), c0)
  assert.equal(JSON.parse(readFileSync(join(work, "package-lock.json"), "utf8")).packages[""].version, "0.1.1")
  reset()
  const recorded = record(first)
  assert.equal(recorded.status, 0, recorded.stderr)
  reset()
  git(work, "pull", "-q", "--ff-only", "origin", "main")
  assert.equal(git(work, "tag", "-l", "v0.1.1"), "v0.1.1")

  // several commits since the previous release: the greatest bump wins, wherever it is
  const c1 = commit("feat: a\n\nRelease-Bump: minor")
  const c2 = commit("fix: b\n\nRelease-Bump: patch\nRelease-Bump: minor")
  git(work, "push", "-q", "origin", "main")
  const second = prepare(c2)
  assert.deepEqual([second.mode, second.version, second.bump, second.baseVersion], ["new", "0.2.0", "minor", "0.1.1"])
  const patchDiff = git(work, "diff")
  reset()

  // main moves and the first push is rejected: the record step re-applies the patch on the new tip
  const c3 = commit("chore: landed during the build", "late.txt")
  git(work, "push", "-q", "origin", "main")
  const hook = join(origin, "hooks", "pre-receive")
  writeFileSync(hook, `#!/bin/sh\nif [ ! -f "$GIT_DIR/rejected-once" ]; then touch "$GIT_DIR/rejected-once"; echo "simulated race" >&2; exit 1; fi\n`)
  chmodSync(hook, 0o755)
  const raced = record(second)
  assert.equal(raced.status, 0, raced.stderr)
  assert.match(raced.stderr, /retrying on the new tip/)
  rmSync(hook)
  git(work, "fetch", "-q", "origin", "main", "--tags")
  const tip = git(work, "rev-parse", "origin/main")
  assert.equal(git(work, "rev-parse", "v0.2.0^{commit}"), tip)
  assert.equal(git(work, "rev-parse", `${tip}^`), c3, "release commit sits on the tip it was pushed against")
  assert.equal(JSON.parse(git(work, "show", `${tip}:package.json`)).version, "0.2.0")
  assert.match(git(work, "show", "-s", "--format=%B", tip), new RegExp(`Release-Source: ${c2}\\nRelease-Bump: minor`))
  // idempotent
  const again = record(second)
  assert.equal(again.status, 0, again.stderr)
  assert.match(again.stdout, /already records/)

  // existing mode (tag exists, e.g. a publish that never happened): same source, same patch, same version
  const existing = prepare(c2)
  assert.deepEqual([existing.mode, existing.version, existing.bump, existing.releaseFiles], ["existing", "0.2.0", "minor", second.releaseFiles])
  assert.equal(git(work, "rev-parse", "HEAD"), c2, "builds start from the source commit, never the release commit")
  assert.equal(git(work, "diff"), patchDiff, "the rebuilt tree is byte-identical to the original release tree")

  // a run queued behind a release does not contain that release commit, and must still work
  const queued = prepare(c3)
  assert.deepEqual([queued.mode, queued.version, queued.bump], ["new", "0.2.1", "patch"])
  // an older source than the latest release is refused
  assert.throws(() => prepare(c1), /not newer than the latest release source/)

  // --- npm shim ---------------------------------------------------------------------------------
  const consumer = join(temporary, "consumer")
  const pkgDir = join(consumer, "node_modules/@s/p")
  const hostDir = join(consumer, `node_modules/@s/p-${process.platform}-${process.arch}`)
  mkdirSync(join(pkgDir, "bin"), { recursive: true })
  mkdirSync(join(hostDir, "bin"), { recursive: true })
  writeFileSync(join(pkgDir, "package.json"), JSON.stringify({ name: "@s/p", type: "module" }))
  copyFileSync(join(repoRoot, "packaging/npm/bin/effect-scan.js"), join(pkgDir, "bin/effect-scan.js"))
  copyFileSync(join(repoRoot, "packaging/npm/bin/glibc.mjs"), join(pkgDir, "bin/glibc.mjs"))
  writeFileSync(join(hostDir, "package.json"), JSON.stringify({ name: `@s/p-${process.platform}-${process.arch}` }))
  const fake = join(hostDir, "bin/effect-scan")
  writeFileSync(
    fake,
    `#!/bin/sh
case "$1" in
  exit7) exit 7 ;;
  echo) shift; echo "$@"; exit 0 ;;
  kill9) kill -KILL $$ ;;
  wait) echo $$ > "$PIDFILE"; trap 'echo term > "$OUT"; exit 0' TERM INT HUP; while :; do sleep 0.05; done ;;
esac
`,
  )
  chmodSync(fake, 0o755)
  const shim = join(pkgDir, "bin/effect-scan.js")

  assert.equal(spawnSync(node, [shim, "exit7"]).status, 7, "exit status passes through")
  assert.equal(spawnSync(node, [shim, "echo", "a b"], { encoding: "utf8" }).stdout, "a b\n", "argv and stdout pass through")
  const killed = spawnSync(node, [shim, "kill9"])
  assert.equal(killed.signal, "SIGKILL", "a child killed by a signal kills the shim with the same signal")

  for (const signal of ["SIGTERM", "SIGINT", "SIGHUP"]) {
    const pidfile = join(temporary, `pid-${signal}`)
    const out = join(temporary, `out-${signal}`)
    const run = spawn(node, [shim, "wait"], { env: { ...process.env, PIDFILE: pidfile, OUT: out }, stdio: "ignore" })
    const closed = new Promise((done) => run.on("close", (code, sig) => done({ code, sig })))
    for (let i = 0; i < 200 && !existsSync(pidfile); i++) await new Promise((done) => setTimeout(done, 25))
    assert.ok(existsSync(pidfile), "child started")
    await new Promise((done) => setTimeout(done, 100))
    const childPid = Number(readFileSync(pidfile, "utf8"))
    run.kill(signal)
    const result = await closed
    assert.equal(readFileSync(out, "utf8").trim(), "term", `${signal} reached the native process`)
    assert.equal(result.code, 0)
    assert.throws(() => process.kill(childPid, 0), "no orphan left behind")
  }

  rmSync(hostDir, { recursive: true })
  const missing = spawnSync(node, [shim, "echo"], { encoding: "utf8" })
  assert.equal(missing.status, 2)
  assert.match(missing.stderr, /no binary for .*is not installed|without glibc|Rosetta/)

  console.log("release self-test ok")
} finally {
  rmSync(temporary, { recursive: true, force: true })
}
