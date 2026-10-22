#!/usr/bin/env node
// Regenerates contracts/api-catalog.json from installed packages + the npm registry.
// Run after changing the pinned effect version: `node dist-dev/scripts/generate-api-catalog.js`.
import { readFileSync, writeFileSync } from "node:fs"
import { buildCatalog, CATALOG_PATH, effectImportRoots } from "../tools/api-catalog.js"

const capabilities = JSON.parse(readFileSync("contracts/effect-capabilities.json", "utf8"))
const catalog = buildCatalog(process.cwd(), { v3LineNames: capabilities.v3LinePackages })
// importRoots is derived: `effect` plus every explicit subpath entry of the effect package.
capabilities.importRoots = effectImportRoots(catalog)
writeFileSync("contracts/effect-capabilities.json", `${JSON.stringify(capabilities, null, 2)}\n`)
writeFileSync(CATALOG_PATH, `${JSON.stringify(catalog, null, 1)}\n`)
console.log(`api catalog written: effect@${catalog.effectVersion}, ${Object.keys(catalog.modules).length} modules, ${Object.keys(catalog.packages).length} v4 packages, ${Object.keys(catalog.v3LinePackages).length} v3-line packages`)
