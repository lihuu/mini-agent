# Minimal Agent V0 Implementation Plan

[中文](implementation-plan.md) · English

**Goal:** implement the one-shot `ma` executable from the agreed design.

**Architecture:** a single Rust source file, plain functions handling arguments, model requests, the Guard, the shell and the loop in turn. No async runtime, SDK or plugin abstraction.

**Tech Stack:** Rust 2024, ureq (synchronous HTTP + rustls), serde_json, libc (Unix process control).

**Spec:** [design.en.md](design.en.md). Executed directly in the current session; the user authorised starting once the requirements were clear, with no further choice of execution mode.

## Global Constraints

- The only tool is `shell` and the only protocol is OpenAI-compatible Chat Completions; permissions are fixed at startup.
- `--write` covers the startup cwd and its descendants only, and the Guard is best-effort.
- Product code lives in `src/main.rs`; release hard limit <10 MB, target <5 MB.

## Review Focus

- Piped input and errors must not pollute stdout.
- Tool output over its limit, or a descendant keeping the pipe open, must not cause a permanent wait.
- Redirections, parent-directory paths and symlinks must not pass the obvious out-of-bounds write checks.
- Malformed arguments, a malformed response or an unknown tool returned by the model must not execute anything by accident.
- Exhausting the step budget must not execute further side effects that cannot be fed back.

## Execution Steps

- [x] Create the real-CLI / mock-HTTP integration tests; run them against an empty implementation and confirm the failures come from missing behaviour.
- [x] Implement argument parsing, input bounds, HTTP / JSON and the single tool definition.
- [x] Write failing tests for the Guard and the shell, then implement the shared Policy, obvious-violation refusals, cwd path checks, process-group timeouts and bounded output.
- [x] Wire up the multi-round loop; integration tests cover assistant/tool pairing, multiple tool execution, refusal feedback, error exits and max_steps.
- [x] Add user documentation and macOS/Linux CI, running fmt, clippy, tests, a release build and the size check.
- [x] Review the implementation against the original requirements and state the verification boundary for this machine, Linux CI and a private model endpoint.

## 2026-10-02 Acceptance Result

| Environment | Tests | Release size | Run |
| --- | --- | --- | --- |
| macOS ARM64, Rust 1.91.1 | 20 / 20 pass; fmt and clippy pass | 1,287,664 bytes | help / version pass |
| One-off Alpine Linux ARM64 container, Rust 1.96.1 | same source snapshot, 20 / 20 pass | 1,446,624 bytes | version passes |

Independent review found and fixed `cd` subshell escapes, `CDPATH`, download write paths, a new-symlink window and surviving tool processes after an interrupt, each verified with a real CLI / shell regression. A read-only reviewer confirmed the fixes had no obvious remaining problem.

The Linux container was deleted automatically and the source was mounted read-only. The GitHub CI configuration was added but has not run remotely; Linux x86_64, other distributions, Windows, builds under Rust 1.85 and a private model endpoint all remain unverified. The known non-sandbox boundaries of the V0 Guard are in the README. No installation, publishing, Git commit or host configuration change was performed.

## 2026-10-02 Streaming and verbose Incremental Acceptance

- Upstream requests default to `stream: true` with synchronous SSE aggregation; a plain JSON response is still supported. Tool arguments are executed once fully received, and a broken stream runs no partial tool.
- `--verbose` / `-v` is off by default; when on it prints model text, rounds, durations, shell commands, output and results to stderr live, while stdout still carries only the complete final answer.
- BOM, LF/CRLF/CR and non-conflicting extension metadata replay are supported; a full stderr pipe may drop optional log lines while tool timeouts still take effect.
- The user-adjusted default `max_steps = 200` is kept; no dependency was added.

| Environment | Tests | Release size | Run |
| --- | --- | --- | --- |
| macOS ARM64, Rust 1.91.1 | 31 / 31 pass; fmt and clippy pass | 1,320,784 bytes | help / version pass |
| One-off Alpine Linux ARM64, Rust 1.96.1 | read-only snapshot matching the current source, 31 / 31 pass | 1,512,160 bytes | version passes |

The tests include model text visible before upstream finishes, tool output visible before the shell exits, a tool still terminating on time under a full stderr pipe, and extension metadata replay with conflict refusal. An independent read-only review confirmed the three boundary problems above were fixed with no new blocking finding. The container was deleted and the source snapshot cleaned up; a real model endpoint and remote CI remain unverified.

## 2026-10-02 Context-Too-Long Trimming Fallback

The user confirmed trimming first, without model summarisation. A normal run only appends to the history; on an explicit context error the oldest history is removed in whole tool turns, always keeping the system instruction, the original task and the newest turn, a replaceable trimming notice is inserted, and the model is retried once. The retry counts towards the existing request limit and processed shell commands are not replayed; with no history left to trim, or a retry that still fails, the program exits. HTTP error bodies are limited to 8 KiB and ordinary 400, 401, 429, 5xx and timeouts do not trigger recovery; an explicit error in JSON/SSE is recognised the same way.

On this macOS ARM64 machine: 40 / 40 CLI integration tests pass, fmt, clippy, the release build and the documentation link/command syntax checks pass; the release is 1,320,800 bytes. New tests cover whole-turn pairing, keeping the newest turn with several tools, no accumulating notice across recoveries, no repeated writes, error recognition, the one-retry boundary, the request limit and no partial tool execution from SSE. No dependency or configuration was added.

This increment has not re-run on Linux, in remote CI or against a real model endpoint; the earlier Linux acceptance covered only the source snapshot as it was then.

An independent read-only review found no blocking issue, and additionally used the existing binary to verify five mock scenarios: a 200 JSON context error, a 422 context error, an HTTP error body over 8 KiB, an ordinary JSON error, and exiting with 500 after a recovery retry.

## 2026-10-03 Guard Globs and CLI Information Flags

- Read-only file commands accept globs inside the working directory, covering `cat *.rs`, `wc -l *.rs`, `ls ./*.rs`, multi-level paths and UTF-8 file names. Before execution the literal path and the expanded candidates are checked for directory/symlink boundaries, with a scan of at most 16384 directory entries per argument. Write targets still use literal paths.
- The meaning of single quotes and of a backslash inside double quotes is preserved; a character range mixed with quoting or escaping is conservatively refused. Operands after `--`, globs beginning with `-` and local file paths containing `://` are checked. A failure to initialise the host locale exits explicitly, so the Guard and the shell cannot end up using different character-matching rules.
- `{}` is handled as an ordinary literal; `find -exec` is still refused, but the error names the specific action, and real shell grouping continues to be refused.
- help / version may appear after the prompt, skipping the values of other options and honouring `--`, without reading configuration or stdin.
- No dependency or configuration was added; the existing connection-config-file feature is kept.

| Environment | Tests | Release size | Run |
| --- | --- | --- | --- |
| macOS ARM64, Rust 1.91.1 | 54 / 54 pass; fmt and clippy pass | 1,337,536 bytes | help / version after the prompt pass |
| One-off Alpine Linux ARM64, Rust 1.96.1 | read-only snapshot matching the current source digest, 53 / 53 pass | 1,512,160 bytes | version passes |

Linux is missing one regression that is specific to this machine: a failure to initialise an invalid libc locale. musl accepts that locale name, and the UTF-8 glob read and out-of-bounds symlink regressions still ran and passed. An independent read-only review plus six read-only glob regressions passed, covering quoted character ranges, a literal path with no match, a missing final component, a trailing `/`, `/.` and `/..` fallbacks. The container was deleted and the temporary source snapshot cleaned up. A real Gemma4 endpoint was not retested and remote CI was not run.
