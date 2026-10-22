import { match } from "ts-pattern"
export const label = (n: number) => match(n).with(1, () => "one").otherwise(() => "many")
