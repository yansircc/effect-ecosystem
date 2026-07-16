import { existsSync, readFileSync } from "node:fs"
import { resolve } from "node:path"
import { findNodeAtLocation, getNodeValue, parseTree, printParseErrorCode } from "jsonc-parser"
import { assertRuntimeFacts } from "./contract-validation.js"
import { makeFinding } from "./findings.js"

export function cloudflareRuntimeFacts(root, manifest) {
  if (!manifest) return []
  return manifest.packages
    .filter((pkg) => pkg.wranglerPath)
    .map((pkg) => ({ package: pkg.path, facts: runtimeFactsForPath(root, pkg.wranglerPath) }))
}

function runtimeFactsForPath(root, path) {
  const absolute = resolve(root, path)
  const base = emptyFacts(path)
  if (!existsSync(absolute)) {
    return {
      ...base,
      errors: [errorFact("missing-wrangler-config", `wranglerPath does not exist: ${path}`, path, 1)],
    }
  }

  const text = readFileSync(absolute, "utf8")
  const parsed = parseWranglerConfig(path, text)
  if (parsed.error) {
    return {
      ...base,
      errors: [errorFact(parsed.error.code, parsed.error.message, path, parsed.error.line)],
    }
  }

  const config = parsed.config
  return {
    platform: fact("cloudflare-worker", path, 1),
    compatDate: fact(config.compatibility_date ?? null, path, parsed.lineAt(["compatibility_date"])),
    compatFlags: asArray(config.compatibility_flags).map((value, index) => fact(value, path, parsed.lineAt(["compatibility_flags", index]))),
    entryPoint: fact(config.main ?? null, path, parsed.lineAt(["main"])),
    bindings: bindingFacts(config, path, parsed.lineAt),
    limits: limitFacts(config, path, parsed.lineAt),
    errors: [],
  }
}

export function scanRuntimeFactRules(root, manifest) {
  return cloudflareRuntimeFacts(root, manifest).flatMap((entry) => {
    assertRuntimeFacts(entry.facts)
    return entry.facts.errors.map((item) => makeFinding(root, {
      ruleId: "EFF905",
      file: item.source.path,
      line: item.source.line,
      package: entry.package,
      message: `${item.code}: ${item.message}`,
    }))
  })
}

function emptyFacts(path) {
  return {
    platform: fact("cloudflare-worker", path, 1),
    compatDate: fact(null, path, 1),
    compatFlags: [],
    entryPoint: fact(null, path, 1),
    bindings: [],
    limits: [],
    errors: [],
  }
}

export function parseWranglerConfig(path, text) {
  if (!path.endsWith(".json") && !path.endsWith(".jsonc")) {
    return {
      error: {
        code: "unsupported-wrangler-format",
        message: "Only wrangler.json and wrangler.jsonc are supported runtime fact sources.",
        line: 1,
      },
    }
  }

  const errors: any[] = []
  const tree = parseTree(text, errors, { allowTrailingComma: true, disallowComments: false })
  if (!tree || errors.length > 0) {
    const first = errors[0]
    const code = first ? printParseErrorCode(first.error) : "Unknown"
    return {
      error: {
        code: "invalid-wrangler-config",
        message: `JSONC parse error: ${code}`,
        line: lineForOffset(text, first?.offset ?? 0),
      },
    }
  }

  const config = getNodeValue(tree)
  if (!config || typeof config !== "object" || Array.isArray(config)) {
    return {
      error: {
        code: "invalid-wrangler-config",
        message: "Wrangler JSONC config must be a top-level object.",
        line: 1,
      },
    }
  }

  return {
    config,
    lineAt: (segments) => {
      const node = findNodeAtLocation(tree, segments)
      return node ? lineForOffset(text, node.offset) : 1
    },
  }
}

function bindingFacts(config, path, lineAt) {
  return [
    ...arrayBindings(config.kv_namespaces, "kv_namespace", path, lineAt, ["kv_namespaces"]),
    ...arrayBindings(config.d1_databases, "d1_database", path, lineAt, ["d1_databases"]),
    ...arrayBindings(config.r2_buckets, "r2_bucket", path, lineAt, ["r2_buckets"]),
    ...arrayBindings(config.services, "service", path, lineAt, ["services"]),
    ...arrayBindings(config.durable_objects?.bindings, "durable_object", path, lineAt, ["durable_objects", "bindings"]),
    ...arrayBindings(config.queues?.producers, "queue_producer", path, lineAt, ["queues", "producers"]),
    ...arrayBindings(config.queues?.consumers, "queue_consumer", path, lineAt, ["queues", "consumers"], "queue"),
    ...(config.ai?.binding ? [bindingFact("ai", config.ai, path, lineAt(["ai", "binding"]))] : []),
  ].sort((a, b) => `${a.type}:${a.name}`.localeCompare(`${b.type}:${b.name}`))
}

function arrayBindings(items, type, path, lineAt, segments, nameKey = "binding") {
  return asArray(items)
    .map((item, index) => ({ item, index }))
    .filter(({ item }) => item && typeof item === "object" && item[nameKey])
    .map(({ item, index }) => bindingFact(type, item, path, lineAt([...segments, index, nameKey]), nameKey))
}

function bindingFact(type, item, path, line, nameKey = "binding") {
  return {
    type,
    name: item[nameKey],
    source: source(path, line),
  }
}

function limitFacts(config, path, lineAt) {
  return Object.entries(config.limits ?? {}).map(([name, value]) => ({
    name,
    value,
    source: source(path, lineAt(["limits", name])),
  })).sort((a, b) => a.name.localeCompare(b.name))
}

function fact(value, path, line) {
  return { value, source: source(path, line) }
}

function source(path, line) {
  return { path, line: line || 1 }
}

function errorFact(code, message, path, line) {
  return {
    code,
    message,
    source: source(path, line),
  }
}

function asArray(value) {
  return Array.isArray(value) ? value : []
}

function lineForOffset(text, offset) {
  let line = 1
  for (let index = 0; index < offset && index < text.length; index++) {
    if (text.charCodeAt(index) === 10) line++
  }
  return line
}
