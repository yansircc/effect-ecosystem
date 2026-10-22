import { describe, expect, it } from "@effect/vitest"
import { Effect } from "effect"
import { openTestDb } from "./sqlite.js"

describe("db", () => {
  it.effect("opens", () => Effect.map(openTestDb, (db) => expect(db.path).toBe(":memory:")))
})
