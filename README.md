# Effortline

Effortline is a free, local-first desktop app for understanding endurance training history. It will use a local model to ask useful questions, but code will calculate the numbers and link each claim to source activities. There is no feed, account, subscription, or remote inference service.

The desktop app can preview and save FIT files, import a batch, and answer one saved-activity question: “How has my running changed recently?” Rust compares the median pace of the three newest eligible runs with the three before them and returns source citations. Heart-rate comparison needs enough recorded samples in every run; missing data stays missing. This first answer is deterministic. A local model is not connected yet. General library browsing and backup are not available. Do not put personal activity files in this repository.

## First product slice

M1 will be an Apple Silicon macOS app for early testers. It will import FIT files, answer “Am I running faster at the same effort?” from comparable runs, show the evidence, save the investigation, and support encrypted backup and restore. It will use a free local model with a guided one-time download. Windows and Linux packaging will come later; the core and backup format must stay portable.

## Develop

Install [Rust](https://rust-lang.org/tools/install/), [pnpm](https://pnpm.io/installation), and the [Tauri macOS prerequisites](https://v2.tauri.app/start/prerequisites/). Then run:

```sh
pnpm --dir apps/desktop install
cargo test -p effortline-core
pnpm --dir apps/desktop check
pnpm --dir apps/desktop tauri dev
```

`cargo test -p effortline-core` runs the core FIT importer and encrypted storage tests. These synthetic tests do not establish broad Garmin or COROS compatibility. The save command uses the exact bytes held by the preview. Saving the same file again does not create a duplicate.

Before a code change, run the relevant checks:

```sh
cargo test --workspace --locked
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
pnpm --dir apps/desktop check
pnpm --dir apps/desktop lint
pnpm --dir apps/desktop format:check
pnpm --dir apps/desktop build
```

## Project rules

- [Architecture and product boundaries](docs/engineering/architecture.md)
- [Code guidelines](docs/engineering/code-guidelines.md)
- [Testing strategy](docs/engineering/testing.md)
- [How to make a safe change](docs/engineering/delivery.md)
- [Agent instructions](AGENTS.md)

Effortline is licensed under [GNU GPLv3](LICENSE). Contributions must keep private data, secrets, and personal exports out of the repository.

## Local diagnostic logs

Effortline keeps diagnostic logs on your device to help investigate errors and slow saves. On macOS, open `~/Library/Logs/com.danmurphy.effortline/` in Finder using **Go → Go to Folder**. The files are `diagnostics.jsonl` and `diagnostics.previous.jsonl`, each limited to 1 MiB.

Logs include app version, platform, event times, operation IDs, save-stage timings, sample counts, and error codes. They do not contain activity contents, filenames, routes, or keys. Nothing is uploaded. Review logs before you choose to share them. To clear them, quit Effortline and delete only those two diagnostic files. This does not remove library data. Logs are best effort and may omit records if the disk is unavailable or the app stops suddenly.

The preview checks whether the file is already saved. Existing files show **Already in your library** without a Save button. This check may ask for Keychain access. After the first successful library check or save, the library stays unlocked in memory until the app quits or a storage operation fails. Later saves reuse that session. Fully quit the app before testing a Keychain permission change.
