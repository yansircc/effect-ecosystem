import { Schedule } from "effect"
class C {
  #x = Schedule
  m() { return this.#x.fixed("1 second") }
}
declare const a: any
export const o = a?.b.c
export const p = a!.b.c
export const q = a()?.Schedule.fixed
export const r = Schedule["fixed"]("1 second")
export const s = new (Schedule as any).Foo()
export const t = Schedule.fixed<number>
