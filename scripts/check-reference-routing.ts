#!/usr/bin/env node
import { collectReferenceRoutes, validateReferenceRouting } from "../validator/lib/reference-routing.js"

const failures = validateReferenceRouting(process.cwd())
if (failures.length > 0) {
  for (const failure of failures) console.error(failure)
  process.exit(1)
}

console.log(`reference routing ok (${collectReferenceRoutes(process.cwd()).length} routes)`)
