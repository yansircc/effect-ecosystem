import { RULES } from "./findings.js"
import { sourceFactsForFile } from "./source-facts.js"
import { cloudflareRuntimeFacts } from "./runtime-facts.js"
import { semanticFactsForFile } from "./compiler-facts.js"
import { assertRuntimeFacts, compileContractValidators, signalDefinitionsByKind } from "./contract-validation.js"
import { effectVersionFromResolution } from "./resolver.js"

const SHAPE_PROFILES = {
  "http-server": ["http-server"],
  "http-client": ["http-client"],
  "db:pg": ["db"],
  "db:mysql": ["db"],
  "db:sqlite": ["db"],
  "db:d1": ["db"],
  "db:clickhouse": ["db"],
  ai: ["ai"],
  rpc: ["rpc"],
  workflow: ["workflow"],
  frontend: ["frontend"],
  node: ["node"],
}

const SIGNAL_VALIDATORS = compileContractValidators()
const SIGNAL_DEFINITIONS = signalDefinitionsByKind(SIGNAL_VALIDATORS.signalsContract)

export function buildProfile(root, manifest, files, toolchain, scanState = null, requiredReferences = []) {
  const activeProfiles = activeProfilesFor(manifest)
  const effectVersion = effectVersionFromResolution(scanState)
  return {
    manifestPath: manifest ? ".effect-skill.json" : null,
    activeProfiles,
    effectVersion,
    effectSupport: effectVersion && scanState?.resolution?.effect?.comparison !== "conflict" ? "verified" : "unresolved",
    requiredReferences,
    packages: manifest?.packages.map((pkg) => ({
      path: pkg.path,
      shape: pkg.shape,
      dependencyOwner: pkg.dependencyOwner,
      edges: manifest.executableEdges.filter((edge) => belongsToPackage(pkg.path, edge.path)).map((edge) => edge.path),
      adapters: manifest.allowedAdapters.filter((adapter) => belongsToPackage(pkg.path, adapter.path)).map((adapter) => adapter.path),
      aiProviderTransports: manifest.aiProviderTransports.filter((transport) => belongsToPackage(pkg.path, transport.path)).map((transport) => transport.path),
      tests: files.filter((file) => file.package?.path === pkg.path && file.roles.test).map((file) => file.relative),
      depsResolved: Object.fromEntries(Object.entries(pkg.deps).filter(([name]) => name === "effect" || name.startsWith("@effect/"))),
    })) ?? [],
    hostTooling: manifest?.hostTooling ?? [],
    rules: {
      deterministicFamilies: [...new Set(Object.values(RULES).map((rule) => rule.family))].sort(),
    },
    toolchain: {
      verified: toolchain?.available ?? false,
      typescriptVersion: toolchain?.tscVersion?.match(/Version\s+([^+\s]+)/)?.[1] ?? null,
      tsgoVersion: toolchain?.tsgoVersion ?? null,
    },
  }
}

export function activeProfilesFor(manifest) {
  if (!manifest) return ["core"]
  const profiles = new Set(["core"])
  for (const pkg of manifest.packages) {
    for (const shape of pkg.shape) {
      for (const profile of SHAPE_PROFILES[shape] ?? []) profiles.add(profile)
    }
  }
  return [...profiles].sort()
}

export function requiredReferencesFor(findings = [], signals = []) {
  const references = new Set()
  for (const finding of findings) if (finding.ref) references.add(finding.ref)
  for (const signal of signals) if (signal.skill_ref) references.add(signal.skill_ref)
  return [...references].sort()
}

export function buildSignals(root, manifest, files, scanState = null) {
  const signals = []
  if (!manifest) return signals
  const emit = (kind, partial) => {
    const definition: any = SIGNAL_DEFINITIONS.get(kind)
    if (!definition) throw new Error(`unknown signal kind: ${kind}`)
    const signal = {
      kind,
      ...partial,
      skill_ref: definition.skill_ref,
      agent_action: definition.agent_action,
    }
    const result = SIGNAL_VALIDATORS.validateSignal(signal)
    if (!result.ok) throw new Error(result.message)
    signals.push(signal)
  }
  const runtimeFacts = cloudflareRuntimeFacts(root, manifest)
  for (const entry of runtimeFacts) {
    assertRuntimeFacts(entry.facts)
    emit("cloudflare-runtime-facts", {
      package: entry.package,
      facts: entry.facts,
    })
  }
  const libraryEffectRollups = new Map()
  for (const file of files) {
    if (file.roles.generated) continue
    const sourceFacts = sourceFactsForFile(file)
    const text = sourceFacts.codeLines.join("\n")
    const importsSchema = sourceFacts.imports.some((entry) => entry.specifier === "effect" || entry.specifier === "effect/Schema")
    const facts = semanticFactsForFile(file)
    if (/HttpApiEndpoint|HttpApiGroup|HttpApi\.make/.test(text)) {
      emit("http-api-boundary-file", {
        file: file.relative,
        facts: {
          containsHttpApiToken: true,
          importsSchema,
        },
      })
    }
    if (/Rpc\.make|RpcGroup|RpcRouter/.test(text)) {
      emit("rpc-boundary-file", {
        file: file.relative,
        facts: {
          containsRpcToken: true,
          importsSchema,
        },
      })
    }
    if (file.package?.shape?.includes("library") && /\bexport\s+(const|function)\b/.test(text)) {
      const containsEffectType = /\bEffect\./.test(text)
      const containsWithSpan = /\bEffect\.withSpan\b/.test(text)
      if (containsEffectType) {
        const rollup = libraryEffectRollups.get(file.package.path) ?? { exportedEffectFiles: 0, withSpanFiles: 0, withoutSpanFiles: 0 }
        rollup.exportedEffectFiles += 1
        if (containsWithSpan) rollup.withSpanFiles += 1
        else rollup.withoutSpanFiles += 1
        libraryEffectRollups.set(file.package.path, rollup)
      }
      if (containsEffectType && !containsWithSpan) {
        emit("library-exported-effect-file", {
          package: file.package.path,
          file: file.relative,
          facts: {
            hasExports: true,
            containsEffectType,
            containsWithSpan,
          },
        })
      }
    }
    const resilienceCalls = facts.calls.filter((name) =>
      ["Effect.retry", "Effect.repeat", "Schedule.recurs", "Schedule.exponential", "Schedule.spaced", "Schedule.fixed", "Schedule.jittered", "Schedule.upTo"].includes(name)
    )
    if (resilienceCalls.length > 0 || facts.scheduleMembers.length > 0) {
      emit("resilience-boundary-file", {
        file: file.relative,
        facts: {
          calls: resilienceCalls,
          scheduleMembers: facts.scheduleMembers,
        },
      })
    }
    const pubsubCalls = facts.calls.filter((name) => name.startsWith("PubSub."))
    if (pubsubCalls.length > 0) {
      emit("pubsub-ordering-file", {
        file: file.relative,
        facts: {
          calls: pubsubCalls,
          pubsubConstructors: facts.pubsubConstructors,
        },
      })
    }
  }
  for (const pkg of manifest.packages) {
    const rollup = libraryEffectRollups.get(pkg.path)
    if (pkg.shape.includes("library") && rollup) {
      emit("library-exported-effect-package", {
        package: pkg.path,
        facts: rollup,
      })
    }
    if (pkg.shape.includes("ai")) {
      const effectAiProviderPackages = Object.keys(pkg.deps).filter((name) => name.startsWith("@effect/ai-")).sort()
      emit("ai-runtime-facts", {
        package: pkg.path,
        facts: {
          hasEffectAiDependency: Boolean(pkg.deps["@effect/ai"]),
          effectAiProviderPackages,
          declaredProviderTransports: manifest.aiProviderTransports
            .filter((transport) => belongsToPackage(pkg.path, transport.path))
            .map((transport) => ({ path: transport.path, owner: transport.owner }))
            .sort((a, b) => a.path.localeCompare(b.path)),
          directProviderSdkDependencies: ["openai", "@anthropic-ai/sdk", "@google/genai"].filter((name) => Boolean(pkg.deps[name])),
        },
      })
    }
    if (pkg.shape.some((shape) => !["library", "node-tool"].includes(shape))) {
      const packageFiles = files.filter((file) => file.package?.path === pkg.path)
      const facts = observabilityFacts(pkg, packageFiles)
      if (facts.hasEffectOpenTelemetryDependency
        || facts.opentelemetryImportedModules.length > 0
        || facts.opentelemetryLayerFactories.length > 0) {
        emit("observability-wiring-facts", {
          package: pkg.path,
          facts,
        })
      }
    }
  }
  return signals
}

function belongsToPackage(packagePath, filePath) {
  return packagePath === "." || filePath === packagePath || filePath.startsWith(`${packagePath}/`)
}

function effectMajor(versionRange) {
  if (typeof versionRange !== "string") return null
  const match = versionRange.match(/\d+/)
  if (!match) return null
  const major = Number(match[0])
  return major === 3 || major === 4 ? major : null
}

function observabilityFacts(pkg, packageFiles) {
  const layerFactories = new Set<string>()
  const importedModules = new Set<string>()
  for (const file of packageFiles) {
    const facts = semanticFactsForFile(file)
    for (const factory of facts.opentelemetryLayerFactories) layerFactories.add(factory)
    for (const moduleName of facts.opentelemetryImportedModules) importedModules.add(moduleName)
  }
  const sortedLayerFactories = [...layerFactories].sort()
  return {
    hasEffectOpenTelemetryDependency: Boolean(pkg.deps["@effect/opentelemetry"]),
    hasEffectPlatformDependency: Boolean(pkg.deps["@effect/platform"]),
    opentelemetryImportedModules: [...importedModules].sort(),
    opentelemetryLayerFactories: sortedLayerFactories,
    containsNodeSdkLayer: hasLayerFactory(sortedLayerFactories, "NodeSdk"),
    containsWebSdkLayer: hasLayerFactory(sortedLayerFactories, "WebSdk"),
    containsOtlpLayer: hasLayerFactory(sortedLayerFactories, "Otlp"),
    containsHttpClientToken: packageFiles.some((file) => {
      const facts = sourceFactsForFile(file)
      return /\bHttpClient\b/.test(facts.codeLines.join("\n")) || facts.imports.some((entry) => entry.specifier === "@effect/platform/HttpClient")
    }),
    containsFetchHttpClientLayer: packageFiles.some((file) => /\bFetchHttpClient\.layer\b/.test(sourceFactsForFile(file).codeLines.join("\n"))),
  }
}

function hasLayerFactory(factories, moduleName) {
  return factories.some((factory) => factory === `${moduleName}.layer` || factory.startsWith(`${moduleName}.layer`))
}
