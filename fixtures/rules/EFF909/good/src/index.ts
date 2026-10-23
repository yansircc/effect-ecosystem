declare function work(): void
export const run = () => { try { work() } catch { return } } // eff-ignore EFF001 reason="third-party callback contract"
