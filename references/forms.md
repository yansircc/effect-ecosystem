# Forms

Form libraries are frontend mechanisms, not Effect semantic owners. Do not
infer an Effect violation from a library import or prescribe one form library
for every frontend.

## Ownership

- The selected form state machine owns raw field values, touched state,
  submission state, and reset semantics. Do not mirror those facts in React
  state or another form controller.
- `effect/Schema` owns domain validation and decoded domain values. Adapt that
  schema at the form boundary when the selected library supports Standard
  Schema; do not restate the same validation in component callbacks.
- Effect owns application I/O, cancellation, concurrency, time, and service
  dependencies after submission crosses into the application boundary.
- The UI framework owns event delivery. A React or browser callback is an
  adapter, not an application runtime owner.

## Boundary shape

Keep one directional path:

```text
UI event -> form state machine -> Effect Schema decode -> application Effect
```

Validation failure remains data for the form projection. A successful decode
may enter the application Effect through its declared host or executable edge.
Do not construct an Effect runtime inside a field component or duplicate
submission state outside the form state machine.

## Review checklist

1. Is there exactly one owner for field and submission state?
2. Is domain validation defined once in `effect/Schema`?
3. Does reset preserve the intended distinction between current and default
   values?
4. Does the UI callback delegate rather than owning application I/O?
5. Are cancellation and long-lived resources owned by an Effect `Scope` after
   the application boundary?

These are architecture review questions. Library imports alone are not
mechanical compliance findings.
