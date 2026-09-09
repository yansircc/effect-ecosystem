import { existsSync, readFileSync } from "node:fs"
import { dirname, resolve } from "node:path"
import { fileURLToPath } from "node:url"
import { Ajv2020 } from "ajv/dist/2020.js"
import { DEFAULT_TEST_GLOBS, depsOf, devDepsOf, readJsonFile, toPosix } from "./util.js"
import { makeFinding } from "./findings.js"

export const MANIFEST_NAME = ".effect-skill.json"
const __dirname = dirname(fileURLToPath(import.meta.url))
const MANIFEST_SCHEMA_PATH = resolve(__dirname, "..", "manifest.schema.json")
const manifestSchema = JSON.parse(readFileSync(MANIFEST_SCHEMA_PATH, "utf8"))
const validateManifest = new Ajv2020({ allErrors: true, strict: false }).compile(manifestSchema)

export function loadManifest(
  root,
  { strict = false, workspaceRoot = root, manifestPath = null } = {},
) {
  const relativeManifestPath = toPosix(manifestPath ?? MANIFEST_NAME)
  const path = resolve(root, relativeManifestPath)
  if (!existsSync(path)) {
    return {
      manifest: null,
      warnings: strict ? [] : [`${relativeManifestPath} not found; running syntax-only scanner`],
      findings: strict
        ? [makeFinding(root, {
            ruleId: "EFF000",
            file: relativeManifestPath,
            message: `--strict requires ${relativeManifestPath}`,
          })]
        : [],
    }
  }

  let raw
  try {
    raw = readJsonFile(path)
  } catch (error) {
    return {
      manifest: null,
      warnings: [],
      findings: [makeFinding(root, {
        ruleId: "EFF900",
        file: relativeManifestPath,
        message: `${relativeManifestPath} is not valid JSON: ${error.message}`,
      })],
    }
  }

  const errors = validateManifestSchema(raw, relativeManifestPath)
  if (errors.length > 0) {
    return {
      manifest: null,
      warnings: [],
      findings: errors.map((message) => makeFinding(root, {
        ruleId: "EFF900",
        file: relativeManifestPath,
        message,
      })),
    }
  }

  const manifest = normalizeManifest(root, raw, path, workspaceRoot)
  const strictFindings = strict && allShapesEmpty(manifest)
    ? [makeFinding(root, { ruleId: "EFF903", file: relativeManifestPath })]
    : []

  return { manifest, warnings: [], findings: strictFindings }
}

function validateManifestSchema(raw, relativeManifestPath = MANIFEST_NAME) {
  if (validateManifest(raw)) return []
  return (validateManifest.errors ?? []).map((error) => {
    const location = error.instancePath || "/"
    return `${relativeManifestPath} schema violation at ${location}: ${error.message}`
  })
}

function normalizeManifest(root, raw, manifestPath, workspaceRoot) {
  const packages = Array.isArray(raw.packages)
    ? raw.packages.map((pkg) => normalizePackage(root, workspaceRoot, pkg))
    : [normalizePackage(root, workspaceRoot, {
      path: ".",
      shape: raw.shape,
      wranglerPath: raw.wranglerPath,
    })]

  return {
    path: manifestPath,
    root,
    packages,
    executableEdges: normalizePathItems(raw.executableEdges ?? []),
    allowedAdapters: normalizePathItems(raw.allowedAdapters ?? []),
    aiProviderTransports: normalizePathItems(raw.aiProviderTransports ?? []),
    nativeDependencyBoundaries: raw.nativeDependencyBoundaries ?? [],
    generatedPaths: raw.generatedPaths ?? [],
    hostTooling: raw.hostTooling ?? [],
    gate: {
      notProven: raw.gate?.notProven ?? [],
    },
    wranglerPath: raw.wranglerPath ? toPosix(raw.wranglerPath) : null,
    testGlobs: raw.testGlobs ?? DEFAULT_TEST_GLOBS,
    suppression: raw.suppression ?? {
      lineLevel: { requireReason: true, requireOwner: false, allowExpires: true },
      manifestPath: { requireReason: true, requireOwner: true },
    },
  }
}

function normalizePackage(root, workspaceRoot, pkg) {
  const packagePath = toPosix(pkg.path ?? ".").replace(/\/$/, "") || "."
  const dependencyOwner = pkg.dependencyOwner ?? "package-local"
  const packageRoot = resolve(root, packagePath)
  const dependencyRoot = dependencyOwner === "workspace-root" ? workspaceRoot : packageRoot
  const packageJsonPath = resolve(dependencyRoot, "package.json")
  const packageJson = existsSync(packageJsonPath) ? readJsonFile(packageJsonPath) : {}
  return {
    path: packagePath,
    root: packageRoot,
    shape: pkg.shape ?? [],
    dependencyOwner,
    dependencyOwnerReason: pkg.dependencyOwnerReason ?? null,
    dependencyRoot,
    wranglerPath: pkg.wranglerPath
      ? toPosix(packagePath === "." ? pkg.wranglerPath : `${packagePath}/${pkg.wranglerPath}`)
      : null,
    packageJsonPath,
    packageJson,
    deps: depsOf(packageJson),
    devDeps: devDepsOf(packageJson),
    relativePackageJson: toPosix(packagePath === "." ? "package.json" : `${packagePath}/package.json`),
  }
}

function normalizePathItems(items) {
  return items.map((item) => ({
    ...item,
    path: toPosix(item.path),
  }))
}

function allShapesEmpty(manifest) {
  return manifest.packages.every((pkg) => pkg.shape.length === 0)
}

export function packageForFile(manifest, relativeFile) {
  if (!manifest) return null
  const normalized = toPosix(relativeFile)
  const sorted = [...manifest.packages].sort((a, b) => b.path.length - a.path.length)
  return sorted.find((pkg) => pkg.path === "." || normalized === pkg.path || normalized.startsWith(`${pkg.path}/`)) ?? null
}

export function rootPackageJson(root) {
  const path = resolve(root, "package.json")
  return existsSync(path) ? readJsonFile(path) : {}
}
