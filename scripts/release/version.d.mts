export type ReleaseBump = "major" | "minor" | "patch";

export declare const releaseBumpFromMessage: (message: string) => ReleaseBump;
export declare const bumpVersion: (version: string, bump: ReleaseBump) => string;
