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
pnpm --dir apps/desktop test
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

Keychain-backed native checks must use a consistently signed app. `tauri dev` and the default debug bundle can be ad-hoc signed; macOS may deny their access to a Keychain item created by the signed app. Build a local Apple Development-signed app with:

```sh
APPLE_SIGNING_IDENTITY="Apple Development: …" pnpm --dir apps/desktop tauri:build:signed
```

Open `target/debug/bundle/macos/Effortline.app`. Do not use `tauri dev` to test the real library Keychain entry. The signing script clears macOS provenance attributes from the generated app bundle, signs it with the stable Effortline bundle identifier, and verifies the signature. This is a local development build, not a distribution or notarization workflow.

The packaged debug app was checked with synthetic files: picker cancellation, preview, save, restart, duplicate save, and invalid-file rejection passed. The live Keychain test also passed. Native app checks leave synthetic activities in the local app library.

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

## Save status regression check

The reported persistent spinner was the wait cursor on a disabled Save button after a successful save. The screenshot and native app both showed a completed result. No multi-minute database or file-write stall was reproduced. A Keychain read took 5821.830 ms while permission was pending, then 32.374 ms with access allowed. These were separate native adapter observations, not storage benchmark timings.

Completed saves now replace the action with a checkmark and **Saved to library** or **Already in your library**. Active saves receive typed stage events from the blocking worker. The UI reports the current stage and elapsed time, including a Keychain prompt hint. It reports completion only after the command returns success. No percentage is estimated. Encryption, Keychain permission policy, and sync/commit behavior are unchanged.

Run the synthetic timing check explicitly:

```sh
cargo test -p effortline-core --locked synthetic_save_stage_timings -- --ignored --nocapture
```

A debug run on Apple Silicon used a generated 120,875-byte FIT with 1,121 samples and real SQLCipher/file storage in a temporary directory. Its provider supplies a test secret in memory; it does not measure Keychain access.

| Stage | Milliseconds |
| --- | ---: |
| Library open/recovery | 15.121 |
| FIT parsing | 33.393 |
| Duplicate check | 0.076 |
| Encryption | 14.953 |
| File write/sync | 8.239 |
| Sample inserts | 2.225 |
| Database commit | 1.449 |

These single-run measurements are diagnostic, not performance guarantees. A large existing library can spend longer in recovery. macOS can still wait for the user to answer a Keychain prompt; the UI now identifies that stage.

Regression checks cover progress arriving before secret access returns, no commit event after a failed parse, retry state, and the rendered saved/duplicate UI with no save button or active progress. The workspace run passed 31 tests (two opt-in tests excluded); the core-only run passed 23; five frontend tests passed. The live Keychain test and timing benchmark passed when run explicitly. Rust format and both lint commands, frontend type/lint/format/build checks, and the packaged debug build passed. The updated Apple Development-signed app displayed the Keychain wait stage and then the checkmarked duplicate panel without a Save button. Native Deny/retry and Developer ID/notarization remain unverified as described above.

The 1,121-sample native reproduction fixture has SHA-256 `375a0395818683c6690dc4451edd174ce1af31759f17897b965fff36013c413c`. It and raw local diagnostics stay outside Git. No library data were deleted. The mixed app library must not be reset to remove these fixtures.

## Larger saves, session reuse, and local diagnostics

Run `cargo test -p effortline-core --locked synthetic_large_save_stage_timings -- --nocapture` for a generated 16,705-sample FIT (200,552 bytes), followed by a distinct 16,706-sample FIT (200,564 bytes). The test pre-populates a temporary library with 64 synthetic activities. It checks saved samples and reopen behavior without any wall-clock pass/fail threshold. All fixtures come from the shared synthetic builder.

A debug baseline before session reuse measured open/recovery at 9.148 ms, FIT parsing 149.429 ms, duplicate check 0.076 ms, encryption 25.420 ms, file write/sync 8.031 ms, sample inserts 36.029 ms, and commit 4.922 ms. This benchmark used an in-memory test secret, not Keychain.

The updated Apple Development-signed app saved both generated files. Its new local diagnostic log recorded:

| Stage | First save, 16,705 samples (ms) | Reused session, 16,706 samples (ms) |
| --- | ---: | ---: |
| Keychain access | 22.961 | Not run |
| Library open/recovery | 19.992 | Not run |
| FIT parsing | 157.304 | 151.370 |
| Duplicate check | 0.105 | 0.258 |
| Encryption | 25.369 | 25.449 |
| File write/sync | 9.607 | 28.342 |
| Sample inserts | 35.388 | 39.701 |
| Database commit | 4.272 | 4.650 |
| Save command total | 275.450 | 250.113 |

The first save recorded 17 sample-count events before commit. The second recorded `library_reused` and no Keychain/open/recovery stages. Both native windows reached **Saved to library**. These are observations on this Mac, not performance guarantees. The native sample stage finished too quickly to inspect each intermediate count visually. A deterministic worker handshake test holds actual persistence after 1,024 rows, confirms the typed progress event arrives before completion, then permits the save to finish. Render tests verify visible uncommitted row counts at 1,024, 8,192, and 16,705 rows. The final row count still says **Not yet committed**; only the command result confirms success.

The partial-write failure test aborts SQLCipher inserts after 1,024 rows. It verifies emitted counts, failed sample persistence, no commit stage, rollback, and successful recovery/retry. A desktop session test saves into a pre-populated library, verifies no new secret/open stage on repeat saves, rejects a competing open, and checks recovery after failure and a simulated restart. Existing interrupted-write tests remain in place. This does not prove recovery from physical power loss.

Four read-only review scopes covered correctness, security, scalability, and tests. Two P2 findings were valid and resolved: log rotation now holds a separate cross-process lock, and the existing sample failure test now covers progress before rollback. Queue overflow, closed logging channels, disk errors, concurrent log writers, size bounds, JSON parsing, and macOS file permissions have regression coverage.

Neither sample persistence nor opening this library reproduced a multi-minute save. The repeated recovery work was real and is now removed within a healthy app session; these measurements do not establish it as the cause of the original delay. The earlier completed-button wait cursor was fixed separately. Slow disks, very large libraries at first open, long permission waits, native Deny/retry, and the original multi-minute incident remain unverified. Fully quit before a Keychain Deny test because the new session retains the unlocked connection.

The native synthetic files and diagnostic logs remain outside Git. No library data were deleted. The app library contains both synthetic and other activities; do not reset it to remove these fixtures. Log cleanup is separate: quit the app and remove only `diagnostics.jsonl` and `diagnostics.previous.jsonl` under `~/Library/Logs/com.danmurphy.effortline/`.

Local checks for this follow-up passed: 38 workspace tests (two opt-in tests excluded), 24 core tests, the opt-in live Keychain test, five frontend tests, both Rust lint commands, Rust format, frontend type/lint/format/build checks, packaged debug build, and Apple Development signature verification. Native large-save and same-session repeated-save checks passed. CI results for this revision are tracked on PR #5.

## Preview lookup follow-up

The preview now checks saved status before offering Save. Test cases cover an absent library without Keychain access or directory creation; matching and new sources in a cached library; matching after restart; stale/version-invalid requests; denied access and retry; and a corrupt original. A successful match verifies the original and creates no activity. Render checks cover saved, checking, and unknown states without a Save action. Local workspace tests passed 40 tests, with two opt-in tests excluded; five frontend tests, Rust format/lint, and frontend type/lint/format checks passed.

For the latest manual flow, choose an already-saved file after restart and allow Keychain access if requested. **Already in your library** should appear without pressing Save. The diagnostic operation is `library_check`, with open/recovery on the first existing-library check and reuse on later checks. Denial leaves the preview visible with **Check library again** and no Save action. A new file should offer Save only after the check reports not-present. A new library is still created only by explicit Save.

The updated signed app passed the automatic lookup check after restart: choosing the saved 16,705-sample synthetic FIT directly showed **Already in your library**, with no Save button. Local diagnostics confirmed `library_check_finished: already_present` and zero `save_started` events in that app session. The packaged build and signature verification also passed.
