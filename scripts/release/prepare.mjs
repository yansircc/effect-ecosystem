import { execFileSync } from "node:child_process";
import { appendFileSync, existsSync, readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { bumpVersion, releaseBumpFromMessage } from "./version.mjs";

const root = resolve(import.meta.dirname, "../..");
const sourceSha = process.argv[2];
if (!sourceSha) throw new Error("usage: prepare.mjs <source-sha>");

const git = (args, options = {}) =>
  execFileSync("git", args, {
    cwd: root,
    encoding: "utf8",
    stdio: options.quiet ? ["ignore", "pipe", "ignore"] : ["ignore", "pipe", "inherit"],
  }).trim();

const writeJson = (path, value) => writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`);

git(["cat-file", "-e", `${sourceSha}^{commit}`]);
const existingCommit = git(
  [
    "log",
    "origin/main",
    "--format=%H",
    "--fixed-strings",
    `--grep=Release-Source: ${sourceSha}`,
    "-n",
    "1",
  ],
  { quiet: true },
);

const mode = existingCommit ? "existing" : "new";
if (existingCommit) git(["checkout", "--detach", existingCommit]);
const messageCommit = existingCommit || sourceSha;
const bump = releaseBumpFromMessage(git(["show", "-s", "--format=%B", messageCommit]));

const manifestPath = resolve(root, "package.json");
const manifest = JSON.parse(readFileSync(manifestPath, "utf8"));
if (mode === "new") {
  const previousRelease = git(
    ["log", "origin/main", "--format=%H", "--grep=^Release-Source: ", "-n", "1"],
    { quiet: true },
  );
  if (previousRelease) {
    const releasedManifest = JSON.parse(git(["show", `${previousRelease}:package.json`]));
    if (manifest.version !== releasedManifest.version) {
      throw new Error(
        `package version is CI-owned; expected ${releasedManifest.version}, received ${manifest.version}`,
      );
    }
  }
}

const version = mode === "new" ? bumpVersion(manifest.version, bump) : manifest.version;
const releaseFiles = ["package.json"];
if (mode === "new") {
  manifest.version = version;
  writeJson(manifestPath, manifest);
  for (const filename of ["package-lock.json", "npm-shrinkwrap.json"]) {
    const path = resolve(root, filename);
    if (!existsSync(path)) continue;
    const lock = JSON.parse(readFileSync(path, "utf8"));
    lock.version = version;
    if (lock.packages?.[""]) lock.packages[""].version = version;
    writeJson(path, lock);
    releaseFiles.push(filename);
  }
}

const tag = `v${version}`;
if (mode === "new") {
  try {
    git(["rev-parse", "--verify", `refs/tags/${tag}`], { quiet: true });
    throw new Error(`release tag already exists: ${tag}`);
  } catch (error) {
    if (error instanceof Error && error.message.startsWith("release tag already exists"))
      throw error;
  }
}

const result = {
  mode,
  name: manifest.name,
  bump,
  version,
  tag,
  sourceSha,
  releaseFiles: releaseFiles.join(","),
  existingCommit: existingCommit || undefined,
};
if (process.env.GITHUB_OUTPUT) {
  appendFileSync(
    process.env.GITHUB_OUTPUT,
    Object.entries(result)
      .filter(([, value]) => value !== undefined)
      .map(([key, value]) => `${key}=${value}`)
      .join("\n") + "\n",
  );
}
process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
