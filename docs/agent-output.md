# Agent output (`--output agent`)

## Invariant

What an agent reads after a scan is bounded by the number of distinct actions, not by the
number of occurrences, and every action can be taken without re-reading the rule catalogue.

Measured before this mode (zeroY3, no manifest): the default non-TTY output (gate-json) was
899 KB for 3335 findings, 215 KB of it the same nine rule messages; findings carried a line but
no column; nothing said that deciding scope (which directories are application source) comes
before 3000 rewrites. The same scan as `--output agent` is 12 KB.

## Shape

```
effect-scan FAIL: <errors> errors, <warnings> warnings in <files> files (scanner <buildId>)
next actions, in order:
1. [scope] ...                      only without a manifest: findings by directory
2. <RULE> <name> · <n> errors in <m> files
   <rule message>
   help: - <before>                 rules.json `help`
         + <after>
   see: <reference section>
   <file>:<line>:<column>           examples from the densest files, with a code frame
   <line> | <source line>
          | ^^^^^                   the hit's column range
   also: <file> (<n>), ... +<k> files
review prompts (not blockers):      one line per signal kind
full list: <evidence>/scan-result.json
```

- **Order** is owned by rules.json `actionOrder` (rule families: a finding of an earlier
  family can change or void a later one), then errors before warnings, then larger groups.
- **Bounds**: 2 framed examples and 5 further files per group; groups whose messages are
  instance-specific (EFF900 stale selectors, EFF909 drift, compiler errors) list up to 8
  instances, because each is its own action. The test `size_is_bounded_by_groups_not_occurrences`
  pins that 500x the findings is not 500x the text.
- **Columns** are Unicode scalar values, 1-based, end exclusive, clipped to the first line of
  the node. Source findings carry them (`column`/`endColumn` in raw-json); projection findings
  (manifest, package, edge, compiler) name a line only and get no frame.
- **Code frames** are read from the file, not from the finding's trimmed `lineText`.

## What did not change

gate-json, its schema and `complianceHash` are unchanged: findings there carry no column, so
a gate baseline does not move because of this mode. The non-TTY default stays gate-json;
existing callers that parse stdout keep working, and SKILL.md asks for `--output agent`.

## Not done, and when it becomes required

Help is per rule, not per instance. A try/catch in a host script and one inside an Effect
program get the same `help`; the scope action is what routes the first case today. Selecting
help by the hit's context (module scope, inside `Effect.gen`, inside a boundary region)
needs the selector hits to carry their enclosing context, which they do not. It becomes
required when measured agent sessions show a rule group whose single `help` is the wrong
rewrite for a recurring context.

Messages are Chinese for source rules and English for the rest; unifying them is a content
change to rules.json and moves no structure.
