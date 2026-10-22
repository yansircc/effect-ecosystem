export type ReleaseBump = "major" | "minor" | "patch";

export declare const releaseBumpTrailers: (message: string) => ReleaseBump[];
export declare const greatestBump: (values: readonly ReleaseBump[]) => ReleaseBump;
export declare const releaseBumpFromMessages: (messages: readonly string[]) => {
  bump: ReleaseBump;
  declared: ReleaseBump[];
  conflicts: boolean;
};
export declare const releaseBumpFromMessage: (message: string) => ReleaseBump;
export declare const bumpVersion: (version: string, bump: ReleaseBump) => string;
