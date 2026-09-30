# Architecture and product boundaries

Effortline keeps an athlete's training history on their device. The desktop app will import source files, calculate measures, and let a local model explain evidence. This document defines the intended boundaries. It does not claim that M1 features already exist.

## Ownership

| Owner | May do | Must not do |
| --- | --- | --- |
| Portable Rust core | Parse source files, own canonical activity identity, store derived facts, run deterministic analysis, validate model tool requests, record evidence | Depend on Tauri, Keychain, AppKit, React, or OS-specific paths |
| Tauri desktop shell | Select files, access the OS secret store, supervise the model process, expose typed commands, package the app | Recalculate domain measures or create a second import path |
| React interface | Show import state, chat, evidence, errors, and backup controls | Read raw activity files or the database directly; treat model text as trusted HTML |

The target flow is `source file → validated importer → encrypted original + canonical record → versioned measures → deterministic comparison → evidence IDs → local model explanation → saved investigation`. A later provider adapter must enter at the validated importer. It cannot bypass identity, provenance, or error handling.

## Data and evidence

- The imported file is a source fact. Keep it immutable. A later correction must be a separate, versioned overlay.
- Canonical records are the single read path for activities. Derived measures can be rebuilt from original files. Record the version of every derivation.
- Only deterministic tool output may support a numeric claim. A citation must resolve to a source revision or an explicit missing-source state.
- An investigation is a snapshot. New data may mark it stale, but must not rewrite the old answer without the athlete asking.
- If data are thin, corrupt, or from mixed sensors, show that limit. Do not invent an “easy” run label or a cause of change.

## Local security and portability

M1 targets SQLCipher Community Edition for searchable records, an encrypted object store for source files, and a random library secret held by the macOS Keychain. A portable backup uses a user-set password. The core must request secrets through an interface; it must not know which OS store supplies them. Windows and Linux can later add adapters without replacing the importer, data model, or backup format.

The app may download the chosen model only after a clear user action. After installation, analysis runs locally. Do not add telemetry, silent update checks, or a remote inference fallback.

## Trade-offs

The first app runs only on Apple Silicon macOS. This keeps packaging and quality checks small. The core stays platform-neutral, and CI will compile it on macOS, Windows, and Linux. M1 indexes only measures needed for the first comparison. Keeping encrypted original FIT files lets later tools replay them without a risky early schema expansion.

## Related

- [Testing strategy](testing.md)
- [How to make a safe change](delivery.md)
- [Project status and setup](../../README.md)

## Current desktop save boundary

The desktop shell retains at most one FIT preview, bounded by the core file-size limit. It keeps the validated source bytes in memory and returns a random preview ID with the summary. The version-1 save command accepts that ID and a request version. It cannot accept a filesystem path, file contents, or a secret. Choosing another file or cancelling clears the previous preview. The shell serializes preview and save operations; blocking file, Keychain, and database work runs outside the UI thread.

The library lives in the app's local data directory under `library/`. The macOS adapter uses the default Keychain with service `com.danmurphy.effortline.library.v1` and account `primary`. It adds a random 32-byte key only if no key or library exists. A duplicate Keychain insertion retrieves the existing key. Denied access, malformed keys, and missing keys for existing libraries fail without replacing the secret. The core receives it through `LibrarySecretProvider`.

The macOS-only [`security-framework`](https://docs.rs/security-framework/3.7.0/security_framework/) dependency provides native Keychain calls under MIT or Apache-2.0. Default features are disabled. This avoids custom unsafe OS bindings and adds no network access. Keychain reads and writes are confined to the desktop adapter. The linked system framework adds no separately bundled runtime. Core encryption and SQLCipher dependencies remain platform-neutral; cross-platform storage and signed-app Keychain behavior still require release validation.

Save progress uses a versioned Tauri channel. The core reports fixed storage stages and elapsed durations through a callback; it has no UI or OS dependency. Tauri maps secret access to the Keychain stage. Progress carries no activity bytes, paths, or secret values. A closed progress channel does not abort an in-flight durable save. The single-preview flow marks the activity saved only after its command response; batch mode reports each file outcome only after that file's save result returns.

### Batch FIT import

The desktop shell offers a separate native multi-file picker for up to 32 FIT files per batch. It reads and imports files serially, with one source byte buffer and one canonical importer operation at a time. Each file is capped by the core 16 MiB limit. The source path is opened read-only, and the exact bytes read are encrypted as the immutable original. No source file is moved, changed, or removed. Progress reports the current filename in the UI, save stages and sample counts, and one outcome per selection. The UI shows a batch bar from completed file outcomes and a sample bar from the core's actual row count; other stages use an indeterminate activity bar because they do not report fractional progress. Local diagnostics record only batch counts, fixed save stages, result states, and stable error codes; they do not record filenames or paths.

Batch cancel is cooperative between files. The active file completes its durable save or failure result; the remaining selected files are marked not imported. Cancel in the native picker selects no files and makes no changes. A retry can select the same files: source identity and the library's verified duplicate check return **Already in your library** for prior successes. A batch-level Keychain or library failure stops further storage attempts to avoid repeated prompts or scans; each unprocessed item receives a not-imported result with the stopping error.

This batch adapter accepts local paths and shows basenames. Later folder or provider imports should reuse the same bounded per-source processing and outcome contract, while supplying source bytes and source provenance through an adapter. A basename does not identify a folder or provider source. They should not reuse the native picker, pass provider paths into the core, or add a second canonical importer. Folder recursion, provider requests, ZIP extraction, and aggregate import queues are outside this slice.

### Library session and recovery

Tauri retains one `ActivityLibrary` behind a mutex after the first successful lookup of an existing library or successful save. It holds the core's exclusive library lock for that session. Subsequent saves reuse the connection and derived encryption keys; they do not reload Keychain or repeat the full object recovery scan. The core still owns recovery. A storage failure drops the connection and lock, so the next attempt opens and recovers again. A FIT parse failure happens before mutation and keeps a healthy session for the next batch member. Every new app process also opens and recovers before writing. There is no persistent flag that skips recovery after a crash. File sync and database commit ordering are unchanged.

The library remains unlocked until the app fully quits or a storage operation fails. Locking Keychain after the first unlock does not revoke this in-memory library session. Quit the app before testing Keychain denial, moving the library, or changing its key. A second app process cannot open the same locked library.

### Local diagnostics

The desktop shell owns device-local diagnostics. Startup, preview results, save stages, sample counts, elapsed durations, library reuse, and stable error codes use a typed allowlist. Logs never accept arbitrary strings from imported files or raw error messages. They exclude activity contents, filenames, paths, source hashes, notes, routes, and secrets. The portable core only emits progress through its callback.

Logs stay on the device. There is no telemetry SDK, upload, remote endpoint, or stable user identifier. Sharing a log is an explicit user action outside this save slice. A bounded background queue decouples log writes from the UI and storage. Rotation retains two files up to 1 MiB each; a separate lock serializes log writes across processes. On macOS, the log directory is mode 0700 and files are mode 0600. These are plaintext diagnostics, not encrypted activity records. Queue drops are counted in the next record. Disk errors stop the logger and emit a fixed stderr message without failing a save. Abrupt process exit can lose queued records; this is a diagnostic aid, not an audit journal.

### Saved status in the preview

Once the summary is ready, the UI calls `check_preview_in_library` with the retained preview ID and version. The shell uses the canonical source identity retained with those exact bytes. The core performs an indexed lookup and authenticates only the matched original, without loading sample rows. A missing library returns not-present without creating storage or accessing Keychain. An existing library may need a Keychain prompt during preview; a successful check reuses the same locked session for later checks and saves.

While checking, the preview has no Save action. A confirmed match shows **Already in your library** immediately. Lookup errors leave the saved status unknown and offer **Check library again**. The UI offers Save only after a not-present result. The save command still deduplicates atomically for other clients or retries. Library-check start, progress, and result events use the same local diagnostic safeguards.
