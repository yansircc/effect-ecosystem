export const USAGE_EXIT_CODE = 64;
export const OUTPUT_MODES = ["human", "gate-json", "raw-json"];
export function resolveOutputMode(args, stdoutIsTTY) {
    if (args.includes("--json")) {
        return usageError("--json was removed; use --output gate-json or --output raw-json");
    }
    const value = optionalValue(args, "--output");
    if (value.ok === false)
        return value;
    if (value.value === null) {
        return { ok: true, mode: stdoutIsTTY ? "human" : "gate-json" };
    }
    if (!isOutputMode(value.value)) {
        return usageError(`invalid --output ${value.value}; expected human, gate-json, or raw-json`);
    }
    return { ok: true, mode: value.value };
}
export function optionalValue(args, name) {
    const index = args.indexOf(name);
    if (index === -1)
        return { ok: true, value: null };
    const value = args[index + 1];
    if (!value || value.startsWith("--")) {
        return usageError(`${name} requires a value`);
    }
    return { ok: true, value };
}
function isOutputMode(value) {
    return OUTPUT_MODES.includes(value);
}
function usageError(message) {
    return { ok: false, message, exitCode: USAGE_EXIT_CODE };
}
