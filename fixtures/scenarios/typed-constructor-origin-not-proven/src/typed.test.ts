export function parameter(Ctor: typeof Response) { return new Ctor() }
declare const Ctor: typeof Response
new Ctor()
type Alias = typeof Response
declare const Aliased: Alias
new Aliased()
declare const Choice: typeof Response | typeof Request
new Choice("https://example.invalid")
export function generic<T extends typeof Response>(Ctor: T) { return new Ctor() }
declare function factory(): typeof Response
new (factory())()
