# Effect v3 Notes

Effect v3 is supported with `effect@3.21.4` or later in major 3 and exact
project-local TypeScript 6.0.3. It has no `@effect/tsgo` prerequisite.

The scanner still performs the same manifest, package, source, execution-edge,
and runtime-fact checks. Target compilation remains the target repository's
own typecheck gate; the scanner does not claim an unavailable v4 language
service for a v3 project.
