#!/usr/bin/env node
// One-time bootstrap of npm package names. Trusted publishing (OIDC, what ci.yml uses) can only be
// configured for a package that already exists, so a brand-new name needs one manual publish first.
// This script only WRITES placeholder packages (version 0.0.0, no files, marked effectScanPlaceholder)
// and prints the manual commands; it never publishes and never needs a token.
// `npm view` (read-only, no auth) decides which names are still missing, so an existing package is
// never given a stray 0.0.0.
//
// usage: bootstrap-packages.mjs --out <dir> [--no-check]
import { spawnSync } from "node:child_process"
import { mkdirSync, readFileSync, rmSync, writeFileSync } from "node:fs"
import { join, resolve } from "node:path"
import { PLATFORMS } from "./platforms.mjs"

const root = resolve(import.meta.dirname, "../..")
const args = process.argv.slice(2)
if (!args.includes("--out")) throw new Error("usage: bootstrap-packages.mjs --out <dir> [--no-check]")
const out = resolve(args[args.indexOf("--out") + 1])
const check = !args.includes("--no-check")

const manifest = JSON.parse(readFileSync(join(root, "package.json"), "utf8"))
const candidates = [
  { name: manifest.name },
  ...PLATFORMS.map((platform) => ({ name: `${manifest.name}-${platform.key}`, platform })),
]
const onRegistry = (name) => {
  const view = spawnSync("npm", ["view", name, "name", "--registry", "https://registry.npmjs.org/"], { encoding: "utf8" })
  if (view.status === 0) return true
  if (!view.stderr.includes("E404")) throw new Error(`cannot tell whether ${name} exists:\n${view.stderr}`)
  return false
}

rmSync(out, { recursive: true, force: true })
const created = []
for (const { name, platform } of candidates) {
  if (check && onRegistry(name)) {
    console.log(`${name}: already on the registry, skipped`)
    continue
  }
  const dir = join(out, name.replace(/^@/, "").replace("/", "-"))
  mkdirSync(dir, { recursive: true })
  writeFileSync(
    join(dir, "package.json"),
    `${JSON.stringify(
      {
        name,
        version: "0.0.0",
        description: `Name reservation for ${name}; the real releases are published by CI.`,
        license: manifest.license,
        repository: manifest.repository,
        effectScanPlaceholder: true,
        ...(platform ? { os: [platform.os], cpu: [platform.cpu], ...(platform.libc ? { libc: [platform.libc] } : {}) } : {}),
        files: [],
      },
      null,
      2,
    )}\n`,
  )
  created.push({ name, dir })
}

if (created.length === 0) {
  console.log("nothing to bootstrap")
} else {
  console.log(`\nPlaceholders written under ${out}. Manual steps (never run by this script):`)
  console.log("1. npmjs.com > Access Tokens: create a granular token, 1 day expiry, read+write for the @yansircc scope only.")
  console.log("2. For each directory below (token via env, never in a file in the repo):")
  for (const { name, dir } of created) {
    console.log(`     (cd ${dir} && NODE_AUTH_TOKEN=<token> npm publish --access public --tag bootstrap)   # ${name}`)
  }
  console.log("   `--tag bootstrap` keeps `latest` free for the first real release.")
  console.log("3. Revoke the token. For each package: npmjs.com > Settings > Trusted Publisher > GitHub Actions:")
  console.log(`     owner/repo from package.json (${manifest.repository?.url ?? "repository"}), workflow filename ci.yml, no environment.`)
  console.log("4. Optionally: npm deprecate '<name>@0.0.0' 'placeholder' (requires auth), then re-run the release.")
}
