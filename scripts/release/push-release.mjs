#!/usr/bin/env node
// Records a published release on main: a release commit (the release patch on the tip of main, with
// `Release-Source:`/`Release-Bump:` trailers) and an annotated tag, pushed atomically.
//
// Runs after publishing, so a failure here never leaves a tag without packages. If main moved while the
// release was building, the patch is re-applied to the new tip (it is a pure function of the version, so
// there is nothing to rebase) and the push is retried. Idempotent: a release commit/tag that already
// records this source is accepted.
//
// usage: push-release.mjs --version V --tag T --bump B --source SHA --files a,b
//          [--remote origin] [--branch main] [--attempts 5] [--root <repo>]
import { execFileSync } from "node:child_process"
import { resolve } from "node:path"
import { applyReleasePatch } from "./release-patch.mjs"

const args = process.argv.slice(2)
const option = (name, fallback) => (args.includes(name) ? args[args.indexOf(name) + 1] : fallback)
const required = (name) => {
  const value = option(name)
  if (!value) throw new Error(`push-release: ${name} is required`)
  return value
}
const version = required("--version")
const tag = required("--tag")
const bump = required("--bump")
const source = required("--source")
const expectedFiles = required("--files").split(",").filter(Boolean).sort()
const remote = option("--remote", "origin")
const branch = option("--branch", "main")
const attempts = Number(option("--attempts", "5"))
const root = resolve(option("--root", resolve(import.meta.dirname, "../..")))
const tip = `${remote}/${branch}`

const run = (gitArgs, { allowFailure = false } = {}) => {
  try {
    return execFileSync("git", gitArgs, { cwd: root, encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }).trim()
  } catch (error) {
    if (allowFailure) return null
    throw new Error(`git ${gitArgs.join(" ")} failed:\n${error.stderr || error.message}`)
  }
}
const sleep = (ms) => new Promise((done) => setTimeout(done, ms))

const recordsSource = (commit) => run(["show", "-s", "--format=%B", commit]).split("\n").includes(`Release-Source: ${source}`)

async function attempt() {
  run(["fetch", "--no-tags", remote, `+refs/heads/${branch}:refs/remotes/${remote}/${branch}`])
  const remoteTag = run(["ls-remote", "--tags", remote, `refs/tags/${tag}^{}`, `refs/tags/${tag}`])
  if (remoteTag) {
    run(["fetch", "--no-tags", remote, `+refs/tags/${tag}:refs/tags/${tag}`])
    if (!recordsSource(`${tag}^{commit}`)) throw new Error(`tag ${tag} already exists and does not record source ${source}`)
    console.log(`${tag} already records ${source}; nothing to push`)
    return true
  }
  const recorded = run(["log", tip, "--format=%H", "--fixed-strings", `--grep=Release-Source: ${source}`, "-n", "1"])
  let commit = recorded
  if (!commit) {
    run(["checkout", "--detach", "--force", tip])
    const files = applyReleasePatch(root, version).filter((file) => run(["diff", "--name-only", "--", file]))
    if (files.sort().join(",") !== expectedFiles.join(",")) {
      throw new Error(`release patch changes ${files.join(",") || "nothing"} on ${tip}; expected ${expectedFiles.join(",")}`)
    }
    run(["config", "user.name", "github-actions[bot]"])
    run(["config", "user.email", "41898282+github-actions[bot]@users.noreply.github.com"])
    run(["add", "--", ...files])
    run(["commit", "-m", `chore(release): v${version}`, "-m", `Release-Source: ${source}\nRelease-Bump: ${bump}`])
    commit = run(["rev-parse", "HEAD"])
  }
  run(["tag", "-f", "-a", tag, "-m", tag, commit])
  const refs = recorded ? [`refs/tags/${tag}`] : [`${commit}:refs/heads/${branch}`, `refs/tags/${tag}`]
  const pushed = run(["push", "--atomic", remote, ...refs], { allowFailure: true })
  if (pushed === null) {
    run(["tag", "-d", tag], { allowFailure: true })
    return false
  }
  console.log(`recorded ${tag} at ${commit}`)
  return true
}

for (let n = 1; n <= attempts; n++) {
  if (await attempt()) process.exit(0)
  console.error(`push-release: attempt ${n}/${attempts} rejected (${branch} moved); retrying on the new tip`)
  await sleep(Number(process.env.PUSH_RELEASE_RETRY_MS ?? 3000) * n)
}
console.error(
  `push-release: could not push ${tag} after ${attempts} attempts. The packages for ${tag} ARE published; only the release commit/tag is missing.\n` +
    `Re-run this job (it is idempotent) or push the tag by hand: git checkout ${source} && node scripts/release/prepare.mjs ${source} && commit with trailers "Release-Source: ${source}" "Release-Bump: ${bump}".`,
)
process.exit(1)
