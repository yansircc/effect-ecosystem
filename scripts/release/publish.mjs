#!/usr/bin/env node
// Publishes every tarball in <dir> (platform packages first, the shim last, so the shim never
// resolves to a missing platform package). Idempotent: a version already on the registry must
// have the tarball's integrity, otherwise the run fails.
//
// usage: publish.mjs <dir> [--dry-run]
//   --dry-run  `npm publish --dry-run` per tarball (an already-published version is reported, not an error);
//              no integrity lookup, no provenance, nothing is uploaded.
import assert from "node:assert/strict"
import { execFileSync, spawnSync } from "node:child_process"
import { createHash } from "node:crypto"
import { readFileSync, readdirSync } from "node:fs"
import { join, resolve } from "node:path"

const REGISTRY = "https://registry.npmjs.org/"
// Trusted publishing (OIDC) can only be configured for a package that already exists, so each package
// name needs one manual first publish. Printed whenever that is the likely cause.
const BOOTSTRAP_HINT =
  "Trusted publishing needs the package to exist first. One-time bootstrap (see docs/rust-migration.md, " +
  '"First publish of a new package name"): node scripts/release/bootstrap-packages.mjs --out <dir>, publish the ' +
  "0.0.0 placeholders by hand with a short-lived granular token, then add the trusted publisher " +
  "(GitHub Actions, workflow ci.yml) on npmjs.com for each package."
const exists = (name) => {
  const view = spawnSync("npm", ["view", name, "name", "--json", "--registry", REGISTRY], { encoding: "utf8" })
  if (view.status === 0) return true
  assert.ok(view.stderr.includes("E404"), `could not determine whether ${name} exists:\n${view.stderr}`)
  return false
}
const args = process.argv.slice(2)
const dir = resolve(args.find((arg) => !arg.startsWith("--")) ?? "")
const dryRun = args.includes("--dry-run")
if (!args.some((arg) => !arg.startsWith("--"))) throw new Error("usage: publish.mjs <dir> [--dry-run]")

const manifestOf = (archive) =>
  JSON.parse(execFileSync("tar", ["-xOf", archive, "package/package.json"], { encoding: "utf8" }))
const archives = readdirSync(dir)
  .filter((file) => file.endsWith(".tgz"))
  .map((file) => join(dir, file))
  .map((archive) => ({ archive, manifest: manifestOf(archive) }))
  .sort((a, b) => Number(Boolean(a.manifest.optionalDependencies)) - Number(Boolean(b.manifest.optionalDependencies)))
assert.ok(archives.length > 1, `no release tarballs in ${dir}`)
for (const { archive, manifest } of archives) {
  assert.ok(!manifest.effectScanPlaceholder, `${archive} is a placeholder build and must not be published`)
}

// Before anything is uploaded: a missing package name would otherwise fail midway, after the
// platform packages were published and before the shim, leaving a half release.
const missing = [...new Set(archives.map(({ manifest }) => manifest.name))].filter((packageName) => !exists(packageName))
if (missing.length > 0) {
  const message = `package(s) not on the registry yet: ${missing.join(", ")}.\n${BOOTSTRAP_HINT}`
  if (!dryRun) {
    console.error(message)
    process.exit(1)
  }
  console.warn(`warning: ${message}`)
}

for (const { archive, manifest } of archives) {
  const spec = `${manifest.name}@${manifest.version}`
  if (dryRun) {
    // A pull request packs the current (already released) version; the registry then refuses the
    // publish itself, which says nothing about the tarball. Any other failure is real.
    const dry = spawnSync("npm", ["publish", archive, "--dry-run", "--access", "public", "--registry", REGISTRY], { encoding: "utf8" })
    process.stdout.write(dry.stdout)
    process.stderr.write(dry.stderr)
    assert.ok(dry.status === 0 || dry.stderr.includes("cannot publish over the previously published"), `${spec}: npm publish --dry-run failed`)
    continue
  }
  const local = `sha512-${createHash("sha512").update(readFileSync(archive)).digest("base64")}`
  const view = spawnSync("npm", ["view", spec, "dist.integrity", "--json", "--registry", REGISTRY], { encoding: "utf8" })
  if (view.status === 0) {
    assert.equal(
      JSON.parse(view.stdout),
      local,
      `published ${spec} does not match the verified candidate. A rebuild is not guaranteed to be byte-identical, so recover by ` +
        "re-running only the failed jobs of the original run (artifacts are kept 30 days), not the whole workflow; see docs/rust-migration.md, \"Recovering a failed release\".",
    )
    console.log(`${spec} is already published with the verified candidate.`)
    continue
  }
  assert.ok(view.stderr.includes("E404"), `could not determine whether ${spec} is published:\n${view.stderr}`)
  const publish = spawnSync("npm", ["publish", archive, "--access", "public", "--provenance", "--registry", REGISTRY], { encoding: "utf8" })
  process.stdout.write(publish.stdout)
  process.stderr.write(publish.stderr)
  if (publish.status !== 0) {
    const auth = /ENEEDAUTH|E401|E403|E404|OIDC|trusted publish/i.test(publish.stderr)
    console.error(`${spec}: npm publish failed.${auth ? `\nThis looks like an authentication/authorization failure. ${BOOTSTRAP_HINT}` : ""}`)
    console.error("Packages published before this one stay published; re-running this job continues (idempotent by integrity).")
    process.exit(1)
  }
}
