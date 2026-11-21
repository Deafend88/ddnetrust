# CLAUDE.md

## Goal
A Rust DDNet client in `rust-client/` (Cargo workspace). It must be byte-identical on the
wire and tick-identical in physics to the C++ client in this repo, which is the
specification. It identifies honestly as "DDNet-rs 0.1.0". Plan: `rust-client/docs/PLAN.md`.

## Session start
Read `rust-client/docs/PLAN.md`, `rust-client/docs/PROGRESS.md` and `git log` first.

## Parity first
- Ported game logic reproduces C++ exactly, including float order, int casts and quirks.
  Keep `C++: file:line` comments.
- Suspected C++ bugs go in `rust-client/docs/DECISIONS.md`; keep the C++ behaviour anyway.
- Verify against the real C++ code: golden vectors (`src/tools/rs_golden.cpp`) and parity
  data (`src/tools/rs_parity.cpp` -> `rust-client/tests/golden/`). A step is done only at
  100% identical.
- Fix code, never weaken tests. Network code must not panic on malformed input. No `unsafe`.
- Load assets from the user's DDNet data dir at runtime; don't copy them.

## Style
- Ponytail (full) for planning and all new Rust code; run `/ponytail-review` before each
  commit. Exception: never simplify away exact C++ behaviour.
- Use context7 for third-party crate docs.

## Testing
- Only a local `DDNet-Server` from this repo, bound to `127.0.0.1` with `sv_register 0`,
  run from a scratch dir (see `rust-client/docs/TESTING.md`).
- Never connect to public servers until the user explicitly allows it.

## Workflow
- Work on branch `rust-client`. Make small focused commits.
- After each verified step: `cargo build`, `cargo clippy --all-targets -- -D warnings`,
  `cargo fmt --check` and `cargo test` must all pass. Then commit and push to `origin`
  (the user's fork) only; never push to `upstream`.
- Update `rust-client/docs/PROGRESS.md` before stopping, especially near a usage limit.
