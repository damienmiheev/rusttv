---
name: rust-guidelines
description: Microsoft's Pragmatic Rust Guidelines. Use when writing, reviewing, or refactoring Rust code in this repo — API and error design, panics vs errors, unsafe/FFI (JNI bridge), docs, workspace layout, performance, and lints.
license: MIT
---

# Pragmatic Rust Guidelines (Microsoft)

The full guidelines are in `guidelines.md` next to this file (~3,500 lines). **Don't read it all.** Find the relevant guidelines by heading and read only those sections:

```bash
grep -n "^## " .claude/skills/rust-guidelines/guidelines.md
```

Each guideline has an ID like `M-PANIC-ON-BUG`; cite the ID when it drives a decision or review comment.

## Where to look first

| Task | Guideline IDs |
|---|---|
| Error types (`thiserror` in core, `anyhow` in CLI) | M-APP-ERROR, M-PANIC-ON-BUG, M-PANIC-IS-STOP |
| Public API of `rusttv-core` | M-PUBLIC-DEBUG, M-PUBLIC-DISPLAY, M-REGULAR-FN, M-SHORT-NAMES, M-DONT-LEAK-TYPES, M-IMPL-ASREF, M-IMPL-IO |
| Parsers over ADB output | M-IMPL-IO ("sans IO"), M-DOCUMENTED-MAGIC, M-TAUTOLOGICAL-TESTS |
| Docs and modules | M-CANONICAL-DOCS, M-FIRST-DOC-SENTENCE, M-MODULE-DOCS |
| Workspace / Cargo | M-CARGO-WORKSPACE, M-CRATES-FLAT-FOLDER, M-CRATES-IN-WORKSPACE, M-MSRV |
| JNI bridge (launcher) | M-UNSAFE, M-UNSAFE-IMPLIES-UB, M-UNSOUND, M-FFI-TRANSLATES |
| Logging | M-LOG-STRUCTURED |
| Lints | M-STATIC-VERIFICATION, M-LINT-OVERRIDE-EXPECT |

## Precedence

`CLAUDE.md` and the decisions in `ARCHITECTURE.md` win over these guidelines. Known differences:

- **M-MIMALLOC-APPS, M-TARGET-CPU** — not adopted by default. New dependencies and build flags need benchmark evidence (BENCHMARK_PLAN.md), and `target-cpu=native` would break distributed binaries and Android builds.
- **Async guidelines** (M-ASYNC-STACK-SIZE, M-YIELD-POINTS) — the core is synchronous until MCP needs async (decision D10).

If another guideline seems to conflict with project rules, follow the project rule and mention the conflict.

## Source

`guidelines.md` is an unmodified copy of <https://microsoft.github.io/rust-guidelines/agents/all.txt>
(repo: <https://github.com/microsoft/rust-guidelines>, MIT License, © Microsoft Corporation), downloaded 2026-10-23.
To update: re-download the file and review the diff before committing.
