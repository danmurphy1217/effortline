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
