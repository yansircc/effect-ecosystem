import { Data } from "effect"
export class Boom extends Data.TaggedError("Boom")<{ readonly reason: string }> {}
