import { Effect } from "effect"
import { magicLink } from "better-auth/plugins/magic-link"
magicLink({ sendMagicLink: () => Effect.runPromise(program) })
