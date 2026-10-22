NodeRuntime.runMain(program) // eff-ignore EFF401 reason="legacy edge"
declare const NodeRuntime: { runMain(effect: unknown): void }
declare const program: unknown
