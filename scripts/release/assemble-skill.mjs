#!/usr/bin/env node
// Assembles the single skill directory from the collected platform builds:
//   SKILL.md agents/ references/ contracts/ build-info.json
//   runtime/effect-scan                  launcher (SKILL.md's command path)
//   runtime/<platform>/effect-scan       native binaries
// The scanner embeds its rules and schemas; references/ and contracts/ are agent reading material.
//
// usage: assemble-skill.mjs [--out dist/skill] [--dist dist] [--require-all-platforms]
import { chmodSync, copyFileSync, cpSync, mkdirSync, readdirSync, rmSync } from "node:fs"
import { join, resolve } from "node:path"
import { collectPlatformBuilds, requireAllPlatforms } from "./artifacts.mjs"
import { BINARY_NAME } from "./platforms.mjs"

const root = resolve(import.meta.dirname, "../..")
const args = process.argv.slice(2)
const option = (name, fallback) => (args.includes(name) ? args[args.indexOf(name) + 1] : fallback)
const dist = resolve(root, option("--dist", "dist"))
const out = resolve(root, option("--out", join(dist, "skill")))

const builds = collectPlatformBuilds(dist)
if (args.includes("--require-all-platforms")) requireAllPlatforms(builds)

rmSync(out, { recursive: true, force: true })
mkdirSync(join(out, "runtime"), { recursive: true })
copyFileSync(join(root, "SKILL.md"), join(out, "SKILL.md"))
cpSync(join(root, "agents"), join(out, "agents"), { recursive: true })
cpSync(join(root, "references"), join(out, "references"), { recursive: true })
mkdirSync(join(out, "contracts"))
for (const file of readdirSync(join(root, "contracts"))) {
  cpSync(join(root, "contracts", file), join(out, "contracts", file), { recursive: true })
}
copyFileSync(join(root, "packaging/skill", BINARY_NAME), join(out, "runtime", BINARY_NAME))
chmodSync(join(out, "runtime", BINARY_NAME), 0o755)
for (const build of builds) {
  const dir = join(out, "runtime", build.platform.key)
  mkdirSync(dir)
  copyFileSync(build.binary, join(dir, BINARY_NAME))
  chmodSync(join(dir, BINARY_NAME), 0o755)
}
copyFileSync(join(dist, "platforms", builds[0].platform.key, "build-info.json"), join(out, "build-info.json"))
console.log(`skill assembled: ${out} (platforms: ${builds.map((b) => b.platform.key).join(", ")})`)
