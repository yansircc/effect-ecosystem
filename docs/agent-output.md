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

## Help by context

A syntax hit carries its **context**, the code world of its innermost enclosing function
(`rules::Context`, `context` in raw-json):

| context | where |
|---|---|
| `module` | no enclosing function |
| `function` | a plain function or method, also a plain callback inside `Effect.gen` |
| `effect` | a generator passed to `Effect.gen`, `Effect.fn(...)`, `Effect.fn("name")(...)` |
| `boundary` | an `Effect.promise/tryPromise/sync/try` callback region |

Measured on six local repositories, the hits of the rules whose `help` rewrites in place
(`yield*`, `Effect.try`, `Clock`, ...) sit inside an Effect program almost never:

| rule | hits | module | function | effect | boundary |
|---|---|---|---|---|---|
| EFF004 await | 2237 | 168 | 2069 | 0 | 0 |
| EFF025 throw | 969 | 237 | 704 | 1 | 27 |
| EFF024 new Error | 878 | 227 | 625 | 5 | 21 |
| EFF001 try | 441 | 34 | 404 | 3 | 0 |
| EFF027 process.env | 159 | 59 | 67 | 30 | 3 |

So a single per-rule help was the wrong first step for ~99% of occurrences. The contract now
says which context a rewrite is written for (`help.in`), and what to do first in every
context (`contexts.<name>.label/guidance`, one sentence each). The agent rendering splits
a rule into one action per context only when the hits are not where the help applies;
rules whose help holds anywhere (EFF002/EFF003: convert the function) stay one group.
Guidance and help are printed in full at their first step and referenced after that.
The cost: zeroY3 renders 27 actions instead of 15 (22 KB instead of 12 KB), still bounded
by rules x contexts.
