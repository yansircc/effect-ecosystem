import { Effect } from "effect"
import { magicLink } from "better-auth/plugins/magic-link"
magicLink({ wrongProperty: () => Effect.runPromise(program) })
