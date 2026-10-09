# mini-agent (ma)

**One model. One tool. One loop.**

[中文](README.zh.md) · [English](README.md)

[![Release](https://img.shields.io/github/v/release/lihuu/mini-agent?sort=semver&label=release)](https://github.com/lihuu/mini-agent/releases)
[![Build](https://github.com/lihuu/mini-agent/actions/workflows/release.yml/badge.svg)](https://github.com/lihuu/mini-agent/actions/workflows/release.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue)](LICENSE)
[![Rust 1.85+](https://img.shields.io/badge/rust-1.85%2B-orange)](https://www.rust-lang.org/)
[![Platform](https://img.shields.io/badge/platform-macOS%20%7C%20Linux-lightgrey)](README.md#installation)

`ma` is a one-shot AI agent that runs as a single binary. It takes a task on the command line, calls an OpenAI-compatible Chat Completions model, does the work with the shell tools already on your machine, prints one answer and exits. macOS and Linux are supported.

## What is ma?

`ma` is a one-shot command-line agent for people who already live in a shell. Instead of a session you attach to, you run it like any other Unix command: `ma 'explain this project'`. The task comes from its arguments, and extra context can come from stdin (`git diff | ma 'summarize these changes'`). Both are sent to an OpenAI-compatible Chat Completions endpoint, and the model then runs shell commands with whatever host CLIs are already installed — `git`, `cargo`, `grep`, `curl`. `ma` takes the directory you started it in as its workspace, writes the final answer to stdout and the process log to stderr, so it composes with the rest of your pipeline. Common write operations and shell network commands are refused by default; `--write` and `--net` opt in, and `--skills` loads named `SKILL.md` directories into the prompt. No conversation state is kept between runs.

The agent itself is one binary with no runtime dependency on Rust and no daemon to start. It is not a replacement for a resident coding agent such as Claude Code or aider; it is what you reach for when you want a model available as a single command inside an existing script, a `Makefile`, or a CI step.

## How is ma different from a resident coding agent?

| | `ma` | Resident coding agents (Claude Code, aider) |
| --- | --- | --- |
| Lifecycle | runs one task, then exits | a session you stay attached to |
| Interface | a Unix command; reads stdin, writes stdout | an interactive terminal UI |
| Tools | the shell and CLIs already on your machine | a built-in tool suite: edit, search, browse |
| State | none carried between runs | session history, checkpoints, permission prompts |
| Distribution | one native binary, no language runtime | usually needs a Node.js or Python runtime first |
| Inside a script or CI | `git diff \| ma 'review this'` | awkward, with no stable non-interactive contract |

Use a resident agent when you want something to sit in your terminal for an afternoon and refactor a repository. Use `ma` when you want a model as one more command in a pipeline.

## Installation

Prebuilt binaries are on the [Releases](https://github.com/lihuu/mini-agent/releases) page. Download the one for your platform and unpack it; no Rust toolchain required:

| Platform | Artifact |
| --- | --- |
| macOS (Apple Silicon) | `ma-<version>-aarch64-apple-darwin.tar.gz` |
| Linux (x86_64) | `ma-<version>-x86_64-unknown-linux-gnu.tar.gz` |

The macOS build requires Big Sur (11.0) or later. The Linux build links glibc dynamically and was measured to run on glibc 2.34 and above (Ubuntu 22.04+, Debian 12+); musl distributions such as Alpine are not supported. Once installed, `ma --version` prints the artifact built for that machine along with its runtime requirement. The full measured matrix is in [Releasing](docs/releasing.en.md#measured-platform-support).

```sh
tar -xzf ma-<version>-aarch64-apple-darwin.tar.gz
mv ma-<version>-aarch64-apple-darwin/ma ~/.local/bin/
ma --help
```

Building from source needs Rust 1.85+ and a C compiler:

```sh
git clone https://github.com/lihuu/mini-agent.git
cd mini-agent
./scripts/install.sh
ma --help
```

The script builds a release binary and installs it into `~/.local/bin`. That directory is used because it falls under the XDG user-bin convention and usually precedes `~/.cargo/bin` on `PATH`, so a later command of the same name cannot quietly shadow it. If an older copy is managed by cargo the script removes it; anything cargo does not own is never touched.

`--prefix DIR` changes the install root (the binary lands in `DIR/bin`), `--uninstall` removes it, and `--offline` stops cargo from using the network. The equivalent manual command is `cargo install --path . --locked --root ~/.local`.

Running `ma` does not require Rust; it uses whatever host CLIs the task needs.

Upgrade later with `ma --update`: it downloads the artifact for this platform, runs it once with `--version` to check it, and replaces itself only when the version matches. Any failure leaves the original binary in place. Ordinary runs also check once a day for a newer release and print a single stderr line; `--no-update-check` turns that off.

## Configuring the model

Use an OpenAI-compatible Chat Completions API that supports tool calling. Replace the endpoint, model and key below with your own:

```sh
export BASE_URL='https://your-endpoint.example/v1'
export MODEL='your-model'
export API_KEY='your-key'
```

`BASE_URL` is the API root; `/chat/completions` is appended. Connection settings can also be stored in a config file, see [Usage](docs/usage.en.md#configuration-and-input).

## Usage

Change into the project you want to work on first. `ma` uses the **current directory at startup** as its workspace:

```sh
cd /path/to/your/project
ma -v 'explain the structure of this project'
```

`-v` shows model text and command execution live; the final answer goes to stdout and the process log to stderr.

```sh
# Take supplementary context from stdin
git diff | ma 'summarize these changes'

# Allow writes inside the current directory and its descendants
ma -v --write 'fix the config file'

# Also allow the shell to use the network
ma -v --write --net 'upgrade the project dependencies'

# Load skills by name; separate multiple names with commas
ma --skills code-review,explain 'review and explain these changes'
```

Piped input exists only in the conversation and never lands on disk. When the context is too long the program recovers in order of increasing cost: compact old turns first, then truncate the newest results, and only then truncate the original input; that last step always warns on stderr, because the omitted part can never be read back. See [Usage](docs/usage.en.md#context-too-long-recovery).

By default common write operations and shell network commands are refused; model API requests are always allowed. The built-in Guard is a best-effort command check, **not a security sandbox**; use a container or virtual machine when you need real isolation. See [Security](SECURITY.en.md).

Full options, config file and output contract are in [Usage](docs/usage.en.md), or run `ma --help`. Releasing and platform support are in [Releasing](docs/releasing.en.md).

`--skills` looks for `SKILL.md` by directory name, first under `.agents/skills/` in the current project and then under `~/.agents/skills/`; the project wins, and anything missing or unreadable is skipped. Without the flag no skill is loaded. The full instructions of the selected skills are added to the prompt and referenced files are read on demand; custom directory arguments are not supported yet. See [skills in Usage](docs/usage.en.md#skills).

## Frequently asked questions

### Do I need Rust installed to run `ma`?

No. [Releases](https://github.com/lihuu/mini-agent/releases) ships prebuilt binaries for macOS (Apple Silicon) and Linux (x86_64): unpack one and run it. Rust 1.85+ and a C compiler are needed only to build from source.

### Which models does it work with?

Any endpoint that speaks the OpenAI-compatible Chat Completions API and supports tool calling — a hosted API or a local server. Set `BASE_URL`, `MODEL` and `API_KEY`, either as environment variables or in the config file. See [Usage](docs/usage.en.md#configuration-and-input).

### Can I pipe a file or a diff into it?

Yes. Anything on stdin is added to the conversation, and piped input exists only in the conversation — it never lands on disk. `git diff | ma 'summarize these changes'` is the typical use.

### Does it work on Windows?

macOS (Apple Silicon) and Linux (x86_64) today. The macOS artifact needs Big Sur (11.0) or later; the Linux artifact needs glibc 2.34+ and does not run on musl distributions such as Alpine.

### Is it safe to let it run shell commands?

Writes and network access are refused by default and must be enabled per run with `--write` and `--net`; model API requests are always allowed. The built-in Guard is a best-effort command check, **not a security sandbox** — use a container or virtual machine when you need real isolation. See [Security](SECURITY.en.md).

### Does it keep any state between runs?

No conversation state. Besides its config file, `ma` writes one timestamp so the once-a-day update notice fires at most once; `--no-update-check` disables that check. See [Usage](docs/usage.en.md).

## License

[MIT](LICENSE).
