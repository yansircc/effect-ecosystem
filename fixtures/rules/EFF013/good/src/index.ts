import { Match } from "effect"
export const label = Match.type<number>().pipe(
  Match.when(1, () => "one"),
  Match.orElse(() => "many"),
)
