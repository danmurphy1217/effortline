# Local model for the first investigation

The first chat question remains a deterministic Rust comparison. If the athlete installs the optional model, the Tauri shell may ask it to write a short interpretation of that result. Rust still calculates pace, selects the six activities, labels data limits, and validates the model's evidence references. The model has no database, filesystem, or network tool.

## Model and runtime

- **Model:** [Qwen3-1.7B](https://huggingface.co/Qwen/Qwen3-1.7B), Apache-2.0, using the third-party [bartowski Q4_K_M GGUF](https://huggingface.co/bartowski/Qwen_Qwen3-1.7B-GGUF).
- **Artifact:** `Qwen_Qwen3-1.7B-Q4_K_M.gguf`, pinned to repository revision `dcb19155b962dbb6389f4691a982043a8e651022`, 1,282,439,584 bytes (about 1.28 GB / 1.19 GiB), SHA-256 `72c5c3cb38fa32d5256e2fe30d03e7a64c6c79e668ad84057e3bd66e250b24fb`.
- **Runtime:** [`llama-cpp-2` 0.1.158](https://docs.rs/llama-cpp-2/0.1.158/llama_cpp_2/), with the `metal` feature. It embeds llama.cpp and uses the model's chat template. llama.cpp is [MIT licensed](https://github.com/ggml-org/llama.cpp/blob/master/LICENSE) and supports Apple Silicon Metal. The Rust bindings are MIT or Apache-2.0.
- **Build tools:** Native builds need CMake and the Xcode command line tools to compile llama.cpp.
- **Device scope:** Apple Silicon macOS. Runtime and model weights are used only on this device. Inference does not start a local server or make a network request.

Qwen publishes the base model under Apache-2.0. The chosen quantization is a third-party conversion of that model using llama.cpp; the app pins the exact converted file and verifies its byte count and SHA-256 both after download and before inference. The source repo lists Q4_K_M at about 1.28 GB and recommends it as its default trade-off. It is much smaller than the original 4.07 GB BF16 weights, with some quality loss. This 1.7B model may still follow instructions poorly; the checked vocabulary and citation gate can reject its answer, while the Rust result remains visible.

The app tells users to allow about 1.3 GB of free storage. The artifact is downloaded only after the user selects **Install model**. The UI streams byte progress, supports cancel and retry, and removes only the model artifact. A cancelled or incomplete `.part` file is not used. No prompt, generated text, or activity detail enters local diagnostics.

The macOS debug app built for this change is about 57.9 MiB because the Metal runtime is linked into the app. The model card does not specify a minimum RAM requirement. On this M4 Pro with 48 GiB, the synthetic evaluation measured model-file verification at 45.2 seconds in an unoptimized debug build. Optimizing only the `sha2` dependency in dev and test profiles reduced that check to 2.3 seconds. The first Metal kernel compilation took 15.6 seconds; a warm initialization took 0.046 seconds. Model load took 471 ms and attempted generation took 771 ms in the warm synthetic run. Peak resident memory was about 1.9 GB.

That synthetic run returned `InferenceFailed`, so it did not produce a validated explanation or aggregate grounding/citation scores. The measured Rust result remains the safe fallback. These are single-run local measurements, not performance guarantees. The investigation now streams fixed stage names and elapsed time to the UI and records the same typed timing markers in local diagnostics; it never logs prompts, generated text, citations, activity values, paths, or model contents.

## Evidence boundary and safe display

The Tauri command runs the existing Rust investigation first. The model receives only its bounded JSON result and six evidence rows. Source hashes are replaced with short aliases (`E1`–`E6`) before inference. Returned aliases must match those Rust rows. Rust maps them back to source IDs for the UI. The UI displays the measured comparison separately from a clearly labelled possible interpretation.

The model cannot supply numeric claims. Rust rejects digits, unknown citations, advice or causal phrases, and words outside a narrow running-comparison vocabulary. It also rejects heart-rate language if the deterministic result does not have enough heart-rate coverage. Invalid output produces a clear message; the measured result and its citations remain available. Rust also reports mixed, missing, or consistent FIT device identifiers. FIT sensor identity is unavailable and the UI states that limit directly. The model is not asked to infer a device or sensor effect.

Insufficient-history results stay deterministic and do not invoke the model in the live flow. Sparse-data handling is included in the opt-in synthetic evaluation.

## Checks

Normal CI builds the macOS runtime and tests download validation, citation validation, and safe fallback without downloading weights. To run the synthetic model evaluation after installing the artifact, set `EFFORTLINE_LOCAL_MODEL_PATH` to its local file and run:

```sh
cargo test -p effortline-desktop --locked evaluate_local_model_with_synthetic_investigations -- --ignored --nocapture
```

The evaluation prints verification, model-load, and inference timings. If all cases complete, it also prints aggregate counts for grounding, citation validity, sparse-data response, and the unsafe-advice guard. It does not print or save prompts, outputs, or activity details. A failed inference stops the evaluation and reports the failure without showing generated text.

## Primary sources

- [Qwen3-1.7B model card and license](https://huggingface.co/Qwen/Qwen3-1.7B)
- [Q4_K_M file size and quantization notes](https://huggingface.co/bartowski/Qwen_Qwen3-1.7B-GGUF)
- [Qwen3 support in llama.cpp](https://github.com/QwenLM/Qwen3/blob/main/docs/source/run_locally/llama.cpp.md)
- [llama.cpp Apple Metal build](https://github.com/ggml-org/llama.cpp/blob/master/docs/build.md#metal-build)
