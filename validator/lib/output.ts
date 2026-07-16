export const USAGE_EXIT_CODE = 64

export const OUTPUT_MODES = ["human", "gate-json", "raw-json"] as const

export type OutputMode = typeof OUTPUT_MODES[number]
type UsageError = { ok: false; message: string; exitCode: typeof USAGE_EXIT_CODE }
type OptionalValue = { ok: true; value: string | null } | UsageError
type OutputModeResult = { ok: true; mode: OutputMode } | UsageError

export function resolveOutputMode(args: string[], stdoutIsTTY: boolean): OutputModeResult {
  if (args.includes("--json")) {
    return usageError("--json was removed; use --output gate-json or --output raw-json")
  }
  const value = optionalValue(args, "--output")
  if (value.ok === false) return value
  if (value.value === null) {
    return { ok: true, mode: stdoutIsTTY ? "human" : "gate-json" }
  }
  if (!isOutputMode(value.value)) {
    return usageError(`invalid --output ${value.value}; expected human, gate-json, or raw-json`)
  }
  return { ok: true, mode: value.value }
}

export function optionalValue(args: string[], name: string): OptionalValue {
  const index = args.indexOf(name)
  if (index === -1) return { ok: true, value: null }
  const value = args[index + 1]
  if (!value || value.startsWith("--")) {
    return usageError(`${name} requires a value`)
  }
  return { ok: true, value }
}

function isOutputMode(value: string): value is OutputMode {
  return (OUTPUT_MODES as readonly string[]).includes(value)
}

function usageError(message: string): UsageError {
  return { ok: false, message, exitCode: USAGE_EXIT_CODE }
}
