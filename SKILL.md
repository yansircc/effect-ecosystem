---
name: effect-ecosystem
description: Use when a task directly changes or reviews Effect v4 code, Effect configuration, Layers, runtimes, Schema, or Effect ecosystem APIs, or explicitly requests an Effect compliance scan. Do not trigger merely because a repository depends on Effect or for unrelated UI, prose, CSS, or ordinary TypeScript changes.
---

# Effect Ecosystem Executor

This skill is an execution entrypoint, not the rule owner. Rule facts live in
`contracts/rules.json`; signal and version capability contracts live in
`contracts/signals.schema.json` and `contracts/effect-capabilities.json`.
Generated summaries live in `references/generated/rules-summary.md` and
`references/generated/checklist.md`.

## Invariant

One Effect semantic rule has one owner. Do not hand-edit the same rule into
`SKILL.md`, scanner rules, and checklist text. Update the owner contract first:
`contracts/rules.json`, `contracts/signals.schema.json`, or
`contracts/effect-capabilities.json`. Then run:

```bash
make build
node dist-dev/scripts/generate-derived-docs.js
```

Fast fail instead of fallback. If required substrate is missing or ambiguous,
stop and repair the owner input; do not load broad references or continue with a
compatibility guess.

## Workflow

1. If the task does not touch Effect code, configuration, or an Effect-owned
   boundary, stop using this skill. Do not scan or load references.
2. For relevant work, run the scanner as a delivery gate or when diagnosing an
   Effect failure; it is not a mandatory opening step:

   ```bash
   effect-scan <repo> --strict --output gate-json --evidence <dir>
   ```

3. Record `<dir>/gate-summary.json` as the Effect mechanical compliance gate.
   Keep `<dir>/scan-result.json` as an artifact path instead of pasting raw JSON
   into task evidence.
4. Treat error findings as mechanical blockers. Warning findings are report-only.
   Fix blockers or add an owned, reasoned suppression.
5. Treat signals as review prompts, not verdicts.
6. Load only the exact `path §heading` entries in
   `effect.requiredReferences`. They derive solely from findings and emitted
   signals. An empty array means load nothing.
7. Read `.effect-skill.json` only when a finding concerns manifest ownership,
   runtime facts, profiles, or suppressions.
8. If `effect.requiredReferences` is missing, the installed scanner is stale;
   run `make install` for release and `make verify`. If Effect support is
   unresolved, repair declared intent or installed reality before continuing.
9. If the same friction class lands in project evidence twice, graduate it into
   a skill artifact instead of leaving another project-only note.
10. Before delivery, run the verification gates below.

## Reference Routing

The scanner owns runtime reference routing. Finding refs and signal `skill_ref`
values are projected into `effect.requiredReferences`; read only the named
section, not the whole reference directory.

`activeProfiles` describes the target and does not authorize automatic document
loading. For architecture or API design, select the one task-relevant deep
reference manually. Do not preload generated rule summaries, checklists,
scanner protocol docs, language-service docs, or every active profile.

## Verification

For this skill repository:

```bash
make verify
```

`make install` publishes an immutable `dist-installed/<buildId>/` scanner
artifact and updates the local bin symlink. Do not run it during development
unless releasing the scanner.

The Effect v4 / TypeScript 7 acceptance and pi-chrome performance gates are
part of default `make verify`. They are also available independently:

```bash
make verify-acceptance
make verify-performance
```

For a target project, run its normal tests plus
`effect-scan <repo> --strict --output gate-json --evidence <dir>`.

## Host Tooling Boundary

This repository contains a Node-based scanner under `validator/**`. That code is
host tooling for the skill, not target application code. The target-project
Effect rules apply to repositories being scanned; host tooling compliance is
tracked separately by this repo's own registry and verification scripts.
