# Effortline agent instructions

This is the canonical entry point for coding agents. Read it before changing code. The linked guides hold detailed shared policy. A local module guide may add facts about its module, but it cannot override these rules. If instructions conflict, stop and report the conflict.

## Product boundaries

- Build a free, local-first tool for an athlete's own training history. Do not add a social feed, account requirement, paywall, analytics SDK, remote inference, or hidden network traffic.
- Chat leads the experience. Deterministic Rust tools calculate measures and choose evidence. The model can explain results and propose hypotheses; it cannot turn a guess into a recorded fact or invent a citation.
- Keep original imported files immutable and encrypted once storage exists. Keep source identity, provenance, and derived measures separate. New sources must use one canonical importer.
- Treat FIT files, provider fields, filenames, notes, prompts, and model tool arguments as untrusted input. A model never gets direct database or filesystem access.
- Keep private athlete files, credentials, model traces, and unsanitized recordings out of Git. Do not use personal training data in examples or tests that are committed.
- M1 targets Apple Silicon macOS. Design the core and backup format for later Windows and Linux adapters. Do not add future-platform UI or providers before their need is proven.

## Code boundaries

- `crates/effortline-core/` owns import, canonical records, encryption-independent domain rules, analysis, evidence, and investigation contracts. It must not import Tauri, React, AppKit, Keychain, or platform paths.
- `apps/desktop/src-tauri/` owns the desktop command boundary and OS adapters: file selection, secret storage, model process, windows, and packaging. Commands validate input and call the core. They do not duplicate analysis rules.
- `apps/desktop/src/` owns presentation and user interaction. It uses typed, versioned commands. It does not read the database or activity files directly.
- Give each durable fact one owner. Use explicit types and stable error codes across boundaries. Add a new layer only for a real ownership, security, or reuse need.
- Keep M0 setup honest. A placeholder screen must not claim that import, encryption, or analysis works before it does.

## Required workflow

1. Read [architecture](docs/engineering/architecture.md), [testing](docs/engineering/testing.md), and [delivery](docs/engineering/delivery.md). Read any local guide for the files you change.
2. Inspect nearby code and tests. State the intended behavior and any material uncertainty before a broad change.
3. Make the smallest complete change. Do not copy product code or configuration from another project. Do not add speculative layers or tests.
4. Run checks that match the risk. Review the full diff and scan for secrets or private activity data before a commit or push.
5. Report what changed, what ran, what did not run, and any remaining risk. Never claim a test passed unless it ran.

## Commands

- Rust core tests: `cargo test -p effortline-core`
- Rust core format: `cargo fmt --all --check`
- Rust core lint: `cargo clippy -p effortline-core --all-targets -- -D warnings`
- Desktop Rust check: `cargo check --workspace`
- Desktop type check: `pnpm --dir apps/desktop check`
- Desktop build: `pnpm --dir apps/desktop build`
- Desktop development: `pnpm --dir apps/desktop tauri dev`

These commands are the current baseline. Add behavior tests and wider checks when the relevant feature exists.
