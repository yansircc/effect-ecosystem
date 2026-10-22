const BUMPS = new Set(["major", "minor", "patch"]);

const BUMP_RANK = { patch: 0, minor: 1, major: 2 };
const RELEASE_COMMIT_TRAILER = /^Release-Source:\s*\S+\s*$/im;

// The `Release-Bump:` values declared by one commit message, validated, in order of appearance.
export const releaseBumpTrailers = (message) => {
  const values = String(message)
    .split(/\r?\n/)
    .flatMap((line) => {
      const match = line.match(/^Release-Bump:\s*(\S+)\s*$/i);
      return match?.[1] ? [match[1].toLowerCase()] : [];
    });
  for (const value of values) {
    if (!BUMPS.has(value)) throw new Error(`Release-Bump must be major, minor, or patch; received ${value}`);
  }
  return values;
};

// A release covers every commit since the previous release, so its bump is the greatest bump any of
// them declares (major > minor > patch; no trailer = patch). Repeated or conflicting trailers (a squash
// body concatenates its commits' trailers) are not errors: conflicts resolve to the greatest value and
// are reported in `conflicts`. Release commits themselves (they carry Release-Source) are not input.
export const greatestBump = (values) => values.reduce((a, b) => (BUMP_RANK[b] > BUMP_RANK[a] ? b : a), "patch");

export const releaseBumpFromMessages = (messages) => {
  const declared = new Set(
    messages.filter((message) => !RELEASE_COMMIT_TRAILER.test(String(message))).flatMap(releaseBumpTrailers),
  );
  return {
    bump: greatestBump([...declared]),
    declared: [...declared].sort((a, b) => BUMP_RANK[a] - BUMP_RANK[b]),
    conflicts: declared.size > 1,
  };
};

export const releaseBumpFromMessage = (message) => releaseBumpFromMessages([message]).bump;

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
