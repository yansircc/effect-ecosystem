// The one list of supported platforms. Everything else (platform npm packages, skill runtime/,
// CI matrix, the launcher) is derived from or checked against it.
export const PLATFORMS = [
  { key: "darwin-arm64", os: "darwin", cpu: "arm64", runner: "macos-14" },
  { key: "linux-x64", os: "linux", cpu: "x64", runner: "ubuntu-24.04", libc: "glibc" },
  { key: "linux-arm64", os: "linux", cpu: "arm64", runner: "ubuntu-24.04-arm", libc: "glibc" },
]

export const BINARY_NAME = "effect-scan"

export const platformByKey = (key) => {
  const platform = PLATFORMS.find((candidate) => candidate.key === key)
  if (!platform) throw new Error(`unsupported platform ${key}; supported: ${PLATFORMS.map((p) => p.key).join(", ")}`)
  return platform
}

export const hostPlatform = () => platformByKey(`${process.platform}-${process.arch}`)

// `dist/platforms/<key>/effect-scan` is where a build leaves (and CI collects) a platform binary.
export const platformBinaryPath = (distDir, key) => `${distDir}/platforms/${key}/${BINARY_NAME}`

if (import.meta.url === `file://${process.argv[1]}` && process.argv[2] === "host") {
  process.stdout.write(`${hostPlatform().key}\n`)
}
