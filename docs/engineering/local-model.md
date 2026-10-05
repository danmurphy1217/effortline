# Local model for training chat

The chat accepts free-form messages and keeps a bounded conversation in memory until the app exits or the user starts a new chat. The local model may answer, ask a clarifying question, or select a registered typed Rust tool. The first tool compares recent running. There is no supported-question list, exact-phrase routing, model-defined tool, or general execution loop. Rust calculates training measures and chooses evidence. The model has no database, filesystem, or network tool.

## Model and runtime

- **Model:** [Qwen3-1.7B](https://huggingface.co/Qwen/Qwen3-1.7B), Apache-2.0, using the third-party [bartowski Q4_K_M GGUF](https://huggingface.co/bartowski/Qwen_Qwen3-1.7B-GGUF).
- **Artifact:** `Qwen_Qwen3-1.7B-Q4_K_M.gguf`, pinned to repository revision `dcb19155b962dbb6389f4691a982043a8e651022`, 1,282,439,584 bytes (about 1.28 GB / 1.19 GiB), SHA-256 `72c5c3cb38fa32d5256e2fe30d03e7a64c6c79e668ad84057e3bd66e250b24fb`.
- **Runtime:** [`llama-cpp-4` 0.7.0](https://docs.rs/llama-cpp-4/0.7.0/llama_cpp_4/), with the `metal` feature. It embeds llama.cpp, uses the model's chat template, and exposes llama.cpp's common sampler. llama.cpp is [MIT licensed](https://github.com/ggml-org/llama.cpp/blob/master/LICENSE) and supports Apple Silicon Metal. The Rust bindings are MIT or Apache-2.0.
- **Build tools:** Native builds need CMake and the Xcode command line tools to compile llama.cpp.
- **Device scope:** Apple Silicon macOS. Runtime and model weights are used only on this device. Inference does not start a local server or make a network request.

Qwen publishes the base model under Apache-2.0. The chosen quantization is a third-party conversion of that model using llama.cpp; the app pins the exact converted file and verifies its byte count and SHA-256 after download and on first use in each app process. It reuses that result only while the artifact's filesystem identity is unchanged. Install and removal clear the cache. The source repo lists Q4_K_M at about 1.28 GB and recommends it as its default trade-off. It is much smaller than the original 4.07 GB BF16 weights, with some quality loss. This 1.7B model may still follow instructions poorly; Rust rejects unverified replies and keeps the measured result visible.

The app tells users to allow about 1.3 GB of free storage. The artifact is downloaded only after the user selects **Install model**. The UI streams byte progress, supports cancel and retry, and removes only the model artifact. A cancelled or incomplete `.part` file is not used. No prompt, generated text, or activity detail enters local diagnostics.

All model instructions live in separate Markdown files under `apps/desktop/src-tauri/prompts/`. Rust includes those files at build time. Keep prompt wording out of Rust string literals so it can be reviewed and changed in one place. Follow the [prompt-writing guide](prompt-writing.md) when you change one.

Every model response uses a JSON Schema derived from its Rust response DTO with `schemars`. The schema is passed with the typed chat request; llama.cpp's common sampler constrains generated tokens and accounts for the chat template's generation prefix. Rust still deserializes the response and runs its citation, phase, and safety checks. Prompts describe response behavior but do not repeat JSON shapes. Normal tests compile every response schema to a llama.cpp grammar. The opt-in Apple Silicon test exercises constrained generation with synthetic questions and evidence.

The macOS debug app built for this change is about 57.9 MiB because the Metal runtime is linked into the app. The model card does not specify a minimum RAM requirement. On this M4 Pro with 48 GiB, the synthetic evaluation measured model-file verification at 45.2 seconds in an unoptimized debug build. Optimizing only the `sha2` dependency in dev and test profiles reduced that check to about 2.3 seconds. The first Metal kernel compilation took 15.6 seconds; a warm initialization took about 0.046 seconds. Peak resident memory was about 1.9 GB.

The first synthetic inference check exposed an output-format bug: the model returned JSON with surrounding text, and parsing the full byte buffer rejected it. The runtime now extracts the first balanced JSON object. A prior single-case M4 Pro investigation run passed its grounding and safety checks, with model load, generation, and output parsing taking about 0.57 s, 0.72 s, and 0.02 s for the compared case. These timings are one device run, not a performance guarantee. The later open-ended chat evaluation has not passed; see [Checks](#checks). Invalid or unsupported output falls back to the measured Rust result. Neither prompts, generated text, nor citation IDs enter diagnostics.

The investigation streams fixed stage names and elapsed time to the UI and records the same typed timing markers in local diagnostics. Diagnostics exclude activity values, filenames, paths, prompts, generated text, citations, and model contents.

## Evidence boundary and safe display

The Tauri command accepts typed messages and uses a closed `ModelChatDecision` enum. A separate closed DTO parses model actions and rejects unknown fields and mismatched action fields. The command runs the registered Rust tool only after the model selects it. A typed `ModelInput` enum gives the model only the bounded result and six evidence rows. The six rows use fixed-size arrays; source hashes are replaced with the closed `EvidenceAlias` enum (`E1`–`E6`) before inference. The model does not choose citation IDs. Rust attaches source IDs from the evidence it supplied, so model-generated citation text cannot create or alter a citation. An evidence reply may only add a tentative, general factor to consider. Rust rejects numbers, direct personal comparisons, direct causal wording, and unmarked possibilities in this model text. The UI labels it **Possible interpretation · local model** and shows the measured result separately. Invalid model text falls back to a **Measured summary · Effortline** built from the Rust result.

The model cannot supply numeric claims. Rust rejects digits, unsafe advice, direct personal comparisons, and a small set of unsupported claims. General answers also pass a personal-history check. Rust also reports mixed, missing, or consistent FIT device identifiers. FIT sensor identity is unavailable and the UI states that limit directly. The model does not report device or heart-rate facts; those stay in the measured result.

Insufficient-history results stay deterministic and do not invoke the model in the live flow. Invalid evidence replies use a safe Rust summary. General training guidance remains separate from claims about the athlete's saved history. Model output is not treated as a source of measured facts.

## Checks

Normal CI builds the macOS runtime and tests download validation, citation validation, and safe fallback without downloading weights. To run the synthetic model evaluation after installing the artifact, set `EFFORTLINE_LOCAL_MODEL_PATH` to its local file and run:

```sh
cargo test -p effortline-desktop --locked evaluate_local_model_with_synthetic_investigations -- --ignored --nocapture
```

The evaluation prints verification, model-load, inference timings, and aggregate counts for grounding, citation validity, sparse-data response, and the unsafe-advice guard. It fails if any synthetic case produces an invalid or unsafe explanation. It does not print or save prompts, outputs, or activity details.

The flexible chat evaluation uses only synthetic questions and evidence:

```sh
cargo test -p effortline-desktop --locked evaluate_local_model_with_synthetic_chat_cases -- --ignored --nocapture
```

The latest M4 Pro run with Rust-derived output schemas produced typed results for every attempted chat response. The broader behavior evaluation did not pass: the model selected the Rust tool for 0 of 2 paraphrases, and its follow-up made an unsupported numeric claim. Rust rejected that follow-up; the measured-evidence fallback passed. The ambiguity check and general hypothesis passed, but the general and out-of-scope reply checks did not. This confirms response-shape enforcement, not flexible chat quality. Do not treat flexible model chat as validated.

The synthetic investigation evaluation also did not pass all checks. Typed explanation output parsed for all three cases. Rust accepted 2 of 3 explanations; the third cited an alias that was not present in the supplied evidence. The sparse-data text check passed 0 of 1 cases, and the unsafe-advice guard passed 3 of 3. On this M4 Pro debug run, artifact verification took 2.39 s. The three model calls took 1.19 s, 0.90 s, and 0.50 s, including model load, generation, and output parsing. This is one local run, not a performance guarantee.

Normal Rust tests cover closed typed actions, safe fallback text, staged conversation history, pace-direction checks, unsupported causal language, citation validation, and sparse results without downloading or running model weights.

## Primary sources

- [Qwen3-1.7B model card and license](https://huggingface.co/Qwen/Qwen3-1.7B)
- [Q4_K_M file size and quantization notes](https://huggingface.co/bartowski/Qwen_Qwen3-1.7B-GGUF)
- [Qwen3 support in llama.cpp](https://github.com/QwenLM/Qwen3/blob/main/docs/source/run_locally/llama.cpp.md)
- [llama.cpp Apple Metal build](https://github.com/ggml-org/llama.cpp/blob/master/docs/build.md#metal-build)
