# Frozen station-intent schema fixtures

`live.intent.json` is a hand-written V1 manifest, deliberately **not** generated from the current
structs. Round-tripping a generated fixture would only prove the code agrees with itself; a frozen
file catches the change that actually breaks users — a field rename, a serde tag change, or a
dropped unknown field — at repo-test time instead of at a customer's daemon restart.

`a_field_from_a_future_build` is present on purpose: a V1 daemon must preserve it on rewrite, so a
newer build's state is not silently destroyed by an older one.
