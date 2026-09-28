# Testing strategy

A test earns its place when it catches a plausible, consequential failure that existing checks do not cover. Coverage is a signal, not a quota. Test behavior and durable state, not private call order or a library's own implementation.

## Test at the owning boundary

- Use unit tests for FIT field parsing, identity, matching, units, calculations, tool validation, and error mapping.
- Use integration tests with real SQLCipher and encrypted files for commit, crash recovery, migration, save/reopen, and backup/restore. Do not mock the storage engine for those contracts.
- Use a few end-to-end runs for the full signed-app journey. Do not repeat every business case at every layer.
- Keep model evaluations small and varied. Score numerical claims, evidence IDs, insufficient-data responses, unsafe coaching, and injected instructions separately. A plausible story with wrong evidence fails.
- Label synthetic corruption, disk-full, model-crash, and provider-error fixtures as synthetic.

M0 has no provider API and needs no cassette. When a provider adapter ships, record a small set of real request/response exchanges for distinct contract assumptions. Remove secrets and personal data, inspect each recording, replay it offline in CI, and keep opt-in live checks for current compatibility. Never fabricate a cassette or treat a replay as proof that the provider still works today.

## Current checks

```sh
cargo test -p effortline-core --locked
cargo fmt --all --check
cargo clippy -p effortline-core --all-targets --locked -- -D warnings
cargo clippy --workspace --all-targets --locked -- -D warnings
pnpm --dir apps/desktop check
pnpm --dir apps/desktop lint
pnpm --dir apps/desktop format:check
pnpm --dir apps/desktop build
```

The core crate has synthetic FIT importer tests for valid data, missing fields, and bad input. These tests do not establish compatibility with real Garmin or COROS files. Desktop import is not available yet. The builder will also run a human end-to-end check before the first tester build.

## Related

- [Architecture and product boundaries](architecture.md)
- [How to make a safe change](delivery.md)
- [Project status and setup](../../README.md)
