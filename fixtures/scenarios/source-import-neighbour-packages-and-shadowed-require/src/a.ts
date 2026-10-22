import pathe from "pathe"
import zodLike from "zod-like"
import fsx from "fsx"
const require = (name: string) => name
export const facts = [pathe, zodLike, fsx, require("zod"), require("fs")]
