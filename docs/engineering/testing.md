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
cargo test --workspace --locked
cargo fmt --all --check
cargo clippy -p effortline-core --all-targets --locked -- -D warnings
cargo clippy --workspace --all-targets --locked -- -D warnings
pnpm --dir apps/desktop check
pnpm --dir apps/desktop lint
pnpm --dir apps/desktop format:check
pnpm --dir apps/desktop build
```

The core crate has synthetic FIT importer tests for valid data, missing fields, and bad input. These tests do not establish broad Garmin or COROS compatibility. Storage tests use real SQLCipher and synthetic FIT bytes. They cover save/reopen, duplicate imports, wrong keys, concurrent opens, damaged or missing originals, and cleanup of uncommitted files. These tests simulate interrupted writes; they do not prove power-loss recovery. The desktop app can preview one FIT file and save it through a versioned command. Desktop tests cover stale previews, cancellation, secret-access failure, safe error responses, exact-byte saves after a source file changes, and duplicate retries.

The live Keychain test is excluded from normal CI. Run it on a development Mac:

```sh
cargo test -p effortline-desktop --lib --locked library_save::tests::live_keychain_preview_save_reopen_and_missing_key -- --ignored --exact
```

This test creates a disposable Keychain entry, saves synthetic FIT bytes through the desktop save handler, reopens the encrypted library, and checks that a missing key is not replaced. It deletes its test entry when it ends. It does not access the app's real library key.

The packaged debug app was checked with synthetic files: picker cancellation, preview, save, restart, duplicate save, and invalid-file rejection passed. The live Keychain test also passed. The native app check leaves one synthetic activity in the local app library.

Before a tester build, check the native picker, cancel, preview, save, duplicate save after restart, denied Keychain access, and error messages in the signed app. Unit tests simulate denied secret access; they do not prove the native permission prompt flow.

## Related

- [Architecture and product boundaries](architecture.md)
- [How to make a safe change](delivery.md)
- [Project status and setup](../../README.md)

## Pre-PR validation for the save slice

The local workspace tests, core tests, live Keychain test, Rust format and lint checks, desktop type/lint/format checks, and frontend build passed. The macOS CI job now runs the desktop Rust tests as part of `cargo test --workspace --locked`.

An Apple Development identity is available locally. A copy of the packaged debug app was signed with that identity and hardened runtime. `codesign --verify --deep --strict` passed. The signed app launched and reached the Keychain access prompt when saving the synthetic fixture. The save then completed with the expected duplicate result, confirming that the signed app could retrieve the existing key and unlock the library. The UI automation tool blocks access to macOS SecurityAgent, so it could not select Deny or control the permission dialog. Deny-and-retry behavior remains unverified. This is not a Developer ID distribution or notarization test; no Developer ID Application identity was available in the local signing identity list.

### Manual save check

1. Open the signed app. Choose a FIT file kept outside the repository. Confirm that the preview matches the file.
2. Select **Save to library**. If Keychain asks, select **Deny**. Confirm a key-access error appears and the preview remains available.
3. Select **Save to library** again. Allow Keychain access. Confirm the app reports a saved activity, or an existing activity if the file was saved before.
4. Quit and reopen the same signed app. Choose the same file and save it again. Confirm **No duplicate was added**.
5. Cancel the picker and try an invalid FIT file. Neither action should offer a saved result.

### Synthetic activity cleanup

Native test data are stored outside Git at `~/Library/Application Support/com.danmurphy.effortline/library/`. No FIT files, encrypted objects, or database files are tracked or included in this PR. The disposable live Keychain test uses a separate temporary directory and test entry; it does not remove the app's library data or key.

The original native fixture has SHA-256 `6c7c9331ce56434d772dd9fe0d686a91c1a5418b4f2d3a7213b520bf7fca6bbe`. This identifies the generated fixture, not a personal export. There is no supported per-activity deletion command in this slice. Do not delete an object file, database row, or Keychain entry by hand. A whole-library reset also removes other activities from view; do not use it on a mixed library.

For a library confirmed to contain only disposable test data, quit Effortline and move the entire `library` directory to a dated archive outside the repository. Keep its Keychain entry. Reopening and saving creates a new library with the retained key. This is a reversible whole-library reset, not selective deletion. For a mixed library, keep the data until a targeted removal operation can validate the fixture hash, back up the complete library, and remove its record and object together. No library data were deleted during these checks.
