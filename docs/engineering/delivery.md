# How to make a safe change

This guide keeps a small change complete without adding work that the product does not need.

## Steps

1. Read [agent instructions](../../AGENTS.md) and the [architecture](architecture.md). Identify the owner of the behavior you will change.
2. Inspect the current code and tests. State the user-visible result, failure path, and any contract or migration that changes.
3. Edit the owning module. Keep inputs typed and validate them at the boundary. Preserve original source data and stable error codes.
4. Add only tests for consequential behavior that no stronger existing test protects. Use real storage for durability paths and real recorded provider exchanges only when a provider adapter exists.
5. Run the focused checks, then review the full diff. Scan for credentials, private FIT files, personal data, unsafe logs, and unplanned network access. Do not commit a private fixture to make a test pass.
6. Commit and push only intentional files. Report the result and any check that could not run. Fix a failing check; do not skip it to make CI green.

## Verification

For the current foundation, run the commands in [Testing strategy](testing.md). A product feature is not complete because the scaffold builds. Its own behavior, failure path, and user message must pass the relevant checks.

## Troubleshooting

- If Rust is missing, install it from the [official Rust installer](https://rust-lang.org/tools/install/) and reopen the shell.
- If the desktop build fails before app code runs, check the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for that OS.
- If a private file appears in `git status`, remove it from the staging area and keep it outside the repository. Do not weaken the ignore rules to make the commit easier.

## Related

- [Architecture and product boundaries](architecture.md)
- [Testing strategy](testing.md)
- [Project status and setup](../../README.md)
