const BUMPS = new Set(["major", "minor", "patch"]);

export const releaseBumpFromMessage = (message) => {
  const values = String(message)
    .split(/\r?\n/)
    .flatMap((line) => {
      const match = line.match(/^Release-Bump:\s*(\S+)\s*$/i);
      return match?.[1] ? [match[1].toLowerCase()] : [];
    });
  if (values.length > 1) throw new Error("commit declares more than one Release-Bump trailer");
  const bump = values[0] ?? "patch";
  if (!BUMPS.has(bump)) {
    throw new Error(`Release-Bump must be major, minor, or patch; received ${bump}`);
  }
  return bump;
};

export const bumpVersion = (version, bump) => {
  const match = String(version).match(/^(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/);
  if (!match) throw new Error(`release version must be strict SemVer: ${version}`);
  if (!BUMPS.has(bump)) throw new Error(`unknown release bump: ${bump}`);
  const major = Number(match[1]);
  const minor = Number(match[2]);
  const patch = Number(match[3]);
  const next =
    bump === "major"
      ? [major + 1, 0, 0]
      : bump === "minor"
        ? [major, minor + 1, 0]
        : [major, minor, patch + 1];
  return next.join(".");
};
