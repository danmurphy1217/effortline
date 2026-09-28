# Code guidelines

These rules apply to new and changed code. They make behavior clear without adding a framework before the product needs one. [Architecture](architecture.md) owns module boundaries, [testing](testing.md) owns test selection, and [delivery](delivery.md) owns the change workflow.

## Write clear, typed code

- Use `rustfmt` for Rust and the existing strict TypeScript settings for the interface. Keep formatting changes limited to files you intend to change. Do not silence a compiler or lint error without explaining why a focused fix is not right.
- Give public functions and cross-boundary values explicit types. Use a named request or result type when it makes a contract clearer. Do not pass an untyped JSON object through the app because it is quick to write.
- Name functions for what they do. Use `find_*` when absence is normal, `get_*` when absence is an error, and `list_*` for a collection. Make mutation names say whether they create, update, import, or delete. State what happens on a repeat call.
- Keep each function focused on one decision or operation. Extract code when it gives a rule one owner or makes a hard path easier to read. Do not add an interface, trait, hook, or helper only to hide a few repeated lines.
- Comment on a constraint or trade-off that the code cannot show. Do not narrate the next line. Use names, types, and tests to explain ordinary behavior.

## Make boundaries explicit

- Parse and validate untrusted bytes, paths, provider data, and model arguments at the entry boundary. Keep provider-specific fields inside the importer. Convert them to a documented canonical type before analysis.
- Use one unit and time convention for each stored field. Put the unit in a type or name when confusion is plausible. Do not silently mix meters and miles, seconds and milliseconds, or local time and UTC.
- Define a typed, versioned command contract between the interface and Tauri. The shell maps command input to core calls and core results to safe UI results. Do not put domain rules in command handlers or React components.
- Keep internal data types private until another module needs a stable contract. Do not add a broad shared `utils` module or re-export every type from a package root.
- Treat a database schema, backup, stored investigation, and model tool schema as durable contracts once users can create them. When one changes, state how old data or calls still work, and test that path.

## Handle failure without hiding it

- Return a typed error for an expected failure. Give errors stable codes at the UI boundary; do not make callers parse message text. Keep a human message clear enough to explain what the user can do next.
- Preserve the original cause of an unexpected failure. Catch only the failure a block can handle. Do not turn an unknown error into success, an empty result, or a guessed analysis.
- Use `Result` for fallible Rust operations. Avoid `unwrap`, `expect`, and `panic!` on imported data, stored data, model output, or normal user actions. A proven invariant may use them only with a short reason.
- For partial import or analysis failure, say which work completed and which did not. Never show a partial result as complete.

## Protect local data

- Keep file writes and database transactions short and scoped. Do not hold a transaction while a model runs or a provider request waits. Define what happens if the app closes between steps.
- Give retryable imports and writes a stable identity. Repeating the same operation must not silently create duplicate activities or overwrite an original file. Define duplicate, stale, and conflicting-source behavior before adding an adapter.
- Make schema and backup changes recoverable. Test open and upgrade with existing data, interrupted writes, and restore before shipping a format change. Do not edit a migration that has reached testers.
- Keep work bounded: file size, record count, memory, concurrency, time, and cancellation. A large or damaged export must not freeze the interface or grow without limit.
- Log only safe diagnostics such as an operation ID, stage, count, duration, and stable error code. Do not log activity contents, routes, notes, raw model prompts, credentials, or full file paths. Do not add telemetry by default.

## Add dependencies with intent

- Prefer the standard library and dependencies already in the repo. Add a package when it removes real work or risk. Check its license, maintenance, platform support, binary size, and access to private data.
- Keep provider SDKs, OS APIs, storage engines, and model runtimes behind their owning adapter. The core must still compile on macOS, Windows, and Linux.
- Add automation when it enforces a real contract. Do not add a new formatter, linter, abstraction, or test matrix without a concrete failure it will catch.

These are defaults, not a reason for broad cleanup. If a feature needs an exception, record the reason near the code or in the relevant design note and keep the exception narrow.
