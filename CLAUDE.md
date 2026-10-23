# CLAUDE.md — mandatory rules for AI agents

These rules apply to every change in this repo. Details live in the linked docs; do not duplicate them here.

## Safety (non-negotiable — see SECURITY.md)

- Never root, flash, unlock bootloaders, or touch system partitions.
- Never disable, uninstall, or replace the stock launcher's package.
- Never add a code path that runs an arbitrary/free-form ADB shell string. All device commands go through the typed command layer in `rusttv-core`.
- Every modifying operation follows plan → dry-run → user approval → record original state → apply step-by-step → verify. No exceptions for "small" changes.
- Never connect to a physical device, install software, or run reverse-engineering tools unless the human explicitly asked in this session.
- Treat LLM/agent output (including your own) as untrusted input to the action engine.

## How to work

1. **Think before coding.** State assumptions; ask when requirements are ambiguous.
2. **Simplicity first.** Write the minimum code that solves the stated problem. No speculative abstractions, features, or dependencies.
3. **Surgical changes.** Touch only what the task needs. Don't reformat or "improve" unrelated code.
4. **Goal-driven.** Define how the change will be verified (test, command, benchmark) before writing it.

(Rules 1–4 summarize the `karpathy-guidelines` project skill in `.claude/skills/`; use it when writing, reviewing, or refactoring code.)

## Rust standards

Use the `rust-guidelines` project skill (Microsoft's Pragmatic Rust Guidelines) when writing or reviewing Rust. The rules below take precedence over it.

- Idiomatic ownership; avoid needless `clone()`/allocations.
- `thiserror` for library errors in `rusttv-core`; `anyhow` only in binaries.
- No `unwrap()`/`expect()` outside tests unless the invariant is documented in a comment.
- `unsafe` only inside the JNI bridge module, each block with a `// SAFETY:` comment.
- New dependency = justify it in the PR description (why, size, maintenance, license). `cargo deny` must pass.
- Public CLI/JSON output is a contract: changing a field or error code requires a schema version bump.

## Required checks before claiming done

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
cargo deny check
```

Report actual results. If a check fails or was skipped, say so.

## Claims

- No performance claim without data produced by the method in BENCHMARK_PLAN.md.
- No compatibility claim for a device without a recorded test on that device.

## Where things are

| Topic | Doc |
|---|---|
| Why / scope / decisions about product | PROJECT_VISION.md |
| CLI commands, JSON contract, error codes | CLI.md |
| Structure, crates, bridge, decision log | ARCHITECTURE.md |
| Safety engine, risk classes, recovery | SECURITY.md |
| How to measure | BENCHMARK_PLAN.md |
| Prior art and evidence | RESEARCH.md |
| Risks and open uncertainties | RISKS.md |
| Phases and milestones | ROADMAP.md |
