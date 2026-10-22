import { execFileSync } from "node:child_process"
import { createHash } from "node:crypto"
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  renameSync,
  rmSync,
  statSync,
  symlinkSync,
  writeFileSync,
} from "node:fs"
import { homedir, tmpdir } from "node:os"
import { dirname, join, resolve } from "node:path"
import { fileURLToPath } from "node:url"

// The checkout root, wherever the compiled script lives: nearest ancestor holding benchmarks/targets.json.
const sourceRoot = (() => {
  let current = dirname(fileURLToPath(import.meta.url))
  while (!existsSync(resolve(current, "benchmarks", "targets.json"))) {
    const parent = dirname(current)
    if (parent === current) throw new Error("benchmarks/targets.json not found above the script")
    current = parent
  }
  return current
})()
const contractPath = resolve(sourceRoot, "benchmarks", "targets.json")

export async function prepareBenchmarkTarget(mode: "local" | "github") {
  const contract = JSON.parse(readFileSync(contractPath, "utf8"))
  if (contract.schemaVersion !== 1) throw new Error("benchmark target schemaVersion must be 1")
  assertLocalContract(contract.local)
  assertGithubContract(contract.github)
  return mode === "github" ? prepareGithubTarget(contract.github) : prepareLocalTarget(contract.local)
}

function prepareLocalTarget(contract: { name: string; sourceFiles: number }) {
  const rootPackage = JSON.parse(readFileSync(resolve(sourceRoot, "package.json"), "utf8"))
  const effectVersion = rootPackage.devDependencies?.effect
  if (typeof effectVersion !== "string") throw new Error("benchmark owner package must declare an Effect version")
  const root = mkdtempSync(join(tmpdir(), "effect-scan-local-benchmark-"))
  mkdirSync(join(root, "src"), { recursive: true })
  writeFileSync(join(root, ".effect-skill.json"), JSON.stringify({ shape: ["library"] }, null, 2))
  writeFileSync(join(root, "package.json"), JSON.stringify({
    name: "effect-scan-local-benchmark",
    private: true,
    type: "module",
    dependencies: { effect: effectVersion },
  }, null, 2))
  writeFileSync(join(root, "tsconfig.json"), JSON.stringify({
    compilerOptions: {
      target: "ES2022",
      module: "NodeNext",
      moduleResolution: "NodeNext",
      strict: true,
      skipLibCheck: true,
      plugins: [{ name: "@effect/language-service" }],
    },
    include: ["src/**/*.ts"],
  }, null, 2))
  symlinkSync(resolve(sourceRoot, "node_modules"), join(root, "node_modules"), process.platform === "win32" ? "junction" : "dir")
  for (let index = 0; index < contract.sourceFiles; index += 1) {
    writeFileSync(join(root, "src", `module-${String(index).padStart(3, "0")}.ts`), localSource(index))
  }
  assertSourcePressure(root, contract.sourceFiles, contract.sourceFiles)
  return {
    root,
    provenance: { kind: "generated", name: contract.name, sourceFiles: contract.sourceFiles },
    cleanup: () => rmSync(root, { recursive: true, force: true }),
  }
}

async function prepareGithubTarget(contract: {
  owner: string
  repo: string
  commit: string
  subdirectory: string
  archiveSha256: string
  workspaceFilter: string
  minimumSourceFiles: number
}) {
  const cacheRoot = resolve(process.env.EFFECT_SCAN_BENCHMARK_CACHE ?? join(homedir(), ".cache", "effect-scan", "benchmarks"))
  mkdirSync(cacheRoot, { recursive: true })
  const archivePath = join(cacheRoot, `${contract.owner}-${contract.repo}-${contract.commit}.tar.gz`)
  await ensureArchive(contract, archivePath)
  const checkout = await ensureCheckout(contract, archivePath, cacheRoot)
  const targetRoot = resolve(checkout, contract.subdirectory)
  if (!existsSync(targetRoot) || !statSync(targetRoot).isDirectory()) {
    throw new Error(`benchmark subdirectory is missing: ${contract.subdirectory}`)
  }
  assertSourcePressure(targetRoot, contract.minimumSourceFiles)
  return {
    root: targetRoot,
    provenance: {
      kind: "github",
      repository: `${contract.owner}/${contract.repo}`,
      commit: contract.commit,
      subdirectory: contract.subdirectory,
      archiveSha256: contract.archiveSha256,
    },
    cleanup: () => {},
  }
}

async function ensureArchive(contract: { owner: string; repo: string; commit: string; archiveSha256: string }, archivePath: string) {
  if (existsSync(archivePath) && sha256(archivePath) === contract.archiveSha256) return
  if (existsSync(archivePath)) rmSync(archivePath)
  const url = `https://codeload.github.com/${contract.owner}/${contract.repo}/tar.gz/${contract.commit}`
  const response = await fetch(url)
  if (!response.ok) throw new Error(`benchmark archive download failed: HTTP ${response.status}`)
  const temporaryPath = `${archivePath}.partial-${process.pid}`
  writeFileSync(temporaryPath, Buffer.from(await response.arrayBuffer()))
  const actual = sha256(temporaryPath)
  if (actual !== contract.archiveSha256) {
    rmSync(temporaryPath)
    throw new Error(`benchmark archive hash mismatch: expected ${contract.archiveSha256}, got ${actual}`)
  }
  renameSync(temporaryPath, archivePath)
}

async function ensureCheckout(contract: {
  owner: string
  repo: string
  commit: string
  archiveSha256: string
  workspaceFilter: string
}, archivePath: string, cacheRoot: string) {
  const checkout = join(cacheRoot, `${contract.owner}-${contract.repo}-${contract.commit}`)
  const markerPath = join(checkout, ".effect-scan-benchmark.json")
  const expectedMarker = JSON.stringify({ archiveSha256: contract.archiveSha256, workspaceFilter: contract.workspaceFilter })
  if (existsSync(markerPath) && readFileSync(markerPath, "utf8") === expectedMarker) return checkout

  const staging = mkdtempSync(join(cacheRoot, ".extract-"))
  try {
    execFileSync("tar", ["-xzf", archivePath, "-C", staging], { stdio: "inherit" })
    const roots = readdirSync(staging).filter((name) => statSync(join(staging, name)).isDirectory())
    if (roots.length !== 1) throw new Error(`benchmark archive must contain one root directory, found ${roots.length}`)
    const extractedRoot = join(staging, roots[0])
    execFileSync("pnpm", ["install", "--frozen-lockfile", "--ignore-scripts", "--filter", `${contract.workspaceFilter}...`], {
      cwd: extractedRoot,
      env: process.env,
      stdio: "inherit",
    })
    writeFileSync(join(extractedRoot, ".effect-scan-benchmark.json"), expectedMarker)
    if (existsSync(checkout)) rmSync(checkout, { recursive: true, force: true })
    renameSync(extractedRoot, checkout)
  } finally {
    rmSync(staging, { recursive: true, force: true })
  }
  return checkout
}

function assertGithubContract(contract: {
  owner: string
  repo: string
  commit: string
  subdirectory: string
  archiveSha256: string
  workspaceFilter: string
  minimumSourceFiles: number
}) {
  if (!/^[a-f0-9]{40}$/.test(contract.commit)) throw new Error("GitHub benchmark commit must be a full SHA")
  if (!/^[a-f0-9]{64}$/.test(contract.archiveSha256)) throw new Error("GitHub benchmark archiveSha256 must be SHA-256")
  if (!contract.owner || !contract.repo || !contract.subdirectory || !contract.workspaceFilter) throw new Error("GitHub benchmark contract is incomplete")
  if (!Number.isInteger(contract.minimumSourceFiles) || contract.minimumSourceFiles < 1) throw new Error("minimumSourceFiles must be positive")
}

function assertLocalContract(contract: { name: string; sourceFiles: number }) {
  if (!contract?.name) throw new Error("local benchmark name is required")
  if (!Number.isInteger(contract.sourceFiles) || contract.sourceFiles < 1) {
    throw new Error("local benchmark sourceFiles must be a positive integer")
  }
}

function assertSourcePressure(root: string, minimum: number, exact?: number) {
  const count = countSources(root)
  if (count < minimum) throw new Error(`benchmark pressure weakened: expected at least ${minimum} source files, found ${count}`)
  if (exact !== undefined && count !== exact) throw new Error(`generated benchmark must contain exactly ${exact} source files, found ${count}`)
}

function countSources(root: string): number {
  let count = 0
  for (const entry of readdirSync(root, { withFileTypes: true })) {
    if (entry.name === "node_modules" || entry.name === "dist" || entry.name.startsWith(".")) continue
    const path = join(root, entry.name)
    if (entry.isDirectory()) count += countSources(path)
    else if (/\.(?:[cm]?ts|tsx)$/.test(entry.name)) count += 1
  }
  return count
}

function sha256(path: string) {
  return createHash("sha256").update(readFileSync(path)).digest("hex")
}

function localSource(index: number) {
  return `import { Effect, Schema } from "effect"\n\nexport const Item${index} = Schema.Struct({ id: Schema.Int, label: Schema.String })\nexport const program${index} = Effect.succeed(${index}).pipe(Effect.map((value) => value + 1))\n`
}
