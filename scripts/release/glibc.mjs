// GLIBC symbol-version arithmetic used by check-glibc.mjs.
export const parseVersion = (text) => String(text).split(".").map(Number)
export const compareVersions = (a, b) => {
  const left = parseVersion(a)
  const right = parseVersion(b)
  for (let i = 0; i < Math.max(left.length, right.length); i++) {
    const delta = (left[i] ?? 0) - (right[i] ?? 0)
    if (delta !== 0) return delta
  }
  return 0
}

// Highest `GLIBC_x.y[.z]` named in `objdump -T` output (the dynamic symbol table), or undefined.
export function maxGlibcVersion(objdumpOutput) {
  let max
  for (const match of String(objdumpOutput).matchAll(/\bGLIBC_(\d+(?:\.\d+)+)\b/g)) {
    if (max === undefined || compareVersions(match[1], max) > 0) max = match[1]
  }
  return max
}
