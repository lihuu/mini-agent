# Minimal Agent

**One model. One tool. One loop.**

[中文](README.md) · English

`ma` is a one-shot command-line agent: it takes a task, calls a model, does the work with the shell tools already on the host, prints an answer and exits. macOS and Linux are supported; at runtime it is a single binary.

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

## License

[MIT](LICENSE).
