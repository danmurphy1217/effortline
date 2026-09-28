# Effortline

Effortline is a free, local-first desktop app for understanding endurance training history. It will use a local model to ask useful questions, but code will calculate the numbers and link each claim to source activities. There is no feed, account, subscription, or remote inference service.

The desktop shell and portable Rust core exist. The core now imports FIT activity bytes and has synthetic importer tests. Desktop import, encrypted storage, investigations, and backup are not available yet. Do not put personal activity files in this repository.

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

`cargo test -p effortline-core` runs the core FIT importer tests. These synthetic tests do not establish compatibility with real Garmin or COROS files. The desktop app cannot import files yet.

Before a code change, run the relevant checks:

```sh
cargo test -p effortline-core --locked
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
