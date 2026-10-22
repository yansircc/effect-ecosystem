import { Effect } from "effect"

export interface TestDb {
  readonly path: string
}

export const openTestDb = Effect.sync((): TestDb => ({ path: ":memory:" }))
