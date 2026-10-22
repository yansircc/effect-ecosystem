# CST Store Migration Guide

Use this when a repository already has `.cst/events.jsonl` but current `cst`
commands fail while replaying older events.

## Invariant

`.cst/events.jsonl` remains the task source. A migration is a deliberate store
upgrade, not normal task editing. Do not create a sidecar plan, do not continue
from chat state, and do not add compatibility fallbacks to the scanner or skill.

## Fast-Fail Trigger

Stop and migrate when `cst brief`, `cst add`, or `cst done` fails on historical
event shape, for example:

```text
verifier_contract canonical_source.ref must be git:<sha>:<path>, path@<sha>, or url@<version>
```

## Procedure

1. Save the failing command and stderr as the migration reason.
2. Back up the current store:

   ```bash
   cp .cst/events.jsonl .cst/events.jsonl.pre-migration
   ```

3. Inspect only the failing historical event:

   ```bash
   rg -n '"evidence_kind":"verifier_contract"|canonical_source' .cst/events.jsonl
   ```

4. Rewrite the incompatible evidence payload to the current verifier-contract
   schema. Minimum accepted shape:

   ```json
   {
     "canonical_source": {
       "ref": "<path>@<sha256>",
       "description": "migrated historical verifier contract"
     },
     "contract_artifacts": [{ "path": "<path>", "sha256": "<sha256>" }],
     "verifier_scripts": [{ "path": "<path>", "sha256": "<sha256>" }],
     "manifest": { "path": "<path>", "sha256": "<sha256>", "count": 0 },
     "cheapest_plausible_lie": "<original text>",
     "red_case_runs": [
       {
         "name": "<historical red case>",
         "diff_path": "<path>",
         "diff_sha256": "<sha256>",
         "command": "<command>",
         "expected_exit": 1,
         "observed_exit": 1,
         "stderr_path": "<path>",
         "stderr_sha256": "<sha256>"
       }
     ],
     "blind_spots": [
       {
         "axis": "historical migration",
         "reason": "original event predated verifier_contract schema",
         "review": "preserve original evidence semantics; migrate shape only"
       }
     ]
   }
   ```

5. Re-run:

   ```bash
   cst brief
   ```

6. Record a normal CST note after the store replays:

   ```bash
   cst evidence <root-or-workstream-id> --kind note --summary "migrated historical CST verifier_contract schema"
   ```

## Rules

- Prefer fail-fast migration over reader compatibility. Historical invalid
  events should become valid store data; the current reducer should not learn old
  schemas silently.
- Preserve node ids, event ids, timestamps, task status, and acceptance evidence.
  Change only the invalid payload shape.
- If the migration cannot be made mechanically, stop and report the exact event
  id and validation error.
