#!/usr/bin/env node
import { mkdtempSync, readdirSync, rmSync, writeFileSync } from "node:fs"
import { tmpdir } from "node:os"
import { resolve } from "node:path"
import { spawnSync } from "node:child_process"

const root = mkdtempSync(resolve(tmpdir(), "effect-skill-snippets-"))
try {
  const snippetsDir = resolve(process.cwd(), "references/snippets")
  const snippetFiles = readdirSync(snippetsDir)
    .filter((file) => file.endsWith(".ts"))
    .map((file) => resolve(snippetsDir, file))
  if (snippetFiles.length === 0) {
    console.error("no snippet files found")
    process.exit(1)
  }
  writeFileSync(resolve(root, "tsconfig.json"), JSON.stringify({
    compilerOptions: {
      target: "ES2022",
      module: "NodeNext",
      moduleResolution: "NodeNext",
      strict: true,
      noEmit: true,
      skipLibCheck: true,
      types: []
    },
    files: snippetFiles
  }, null, 2))
  const tsc = resolve(process.cwd(), "node_modules", "typescript", "bin", "tsc")
  const result = spawnSync(process.execPath, [tsc, "--noEmit", "-p", resolve(root, "tsconfig.json")], {
    cwd: process.cwd(),
    encoding: "utf8",
    maxBuffer: 10_000_000,
  })
  if (result.status !== 0) {
    console.error(result.stdout)
    console.error(result.stderr)
    process.exit(result.status ?? 1)
  }
  console.log("snippet typecheck ok")
} finally {
  rmSync(root, { recursive: true, force: true })
}
