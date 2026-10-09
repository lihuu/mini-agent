# Usage Details

[中文](usage.md) · English

Installation and day-to-day use are in the [README](../README.md). The examples below assume `ma` is installed and on `PATH`.

## Build only, without installing

To build inside the source directory only:

```sh
cargo build --release --locked
./target/release/ma --help
```

The artifact is `target/release/ma`. `./target/release/ma` is a path relative to the source directory; from another project you either use the absolute path of the binary or put it on `PATH` first:

```sh
cd /path/to/your/project
/path/to/oneagent/target/release/ma -v 'analyze this project'
```

Replace both paths in the example with real directories. The working directory comes from where you start the binary, not from where the binary lives.

To install onto the user `PATH`, use the script in this repository:

```sh
./scripts/install.sh                     # installs into ~/.local/bin
./scripts/install.sh --prefix ~/.cargo  # or any root; the binary lands in <DIR>/bin
./scripts/install.sh --uninstall        # remove it
```

The script installs into `~/.local/bin` by default. That directory usually precedes `~/.cargo/bin` on `PATH`, so a later command of the same name cannot be quietly shadowed by an old copy. If an older copy is still managed by cargo, the script removes it; files cargo does not manage are never deleted. `--offline` is passed to cargo to stop it using the network (the dependencies must already be in the local cache).

## Updating

`ma --update` downloads the artifact matching this platform from the latest release and replaces itself in place. It accepts only `ma` inside `ma-<version>-<target>.tar.gz`; other members and any path escaping the archive root are refused.

Before replacing anything it writes the candidate binary to a temporary file next to the target and runs it with `--version`; **it replaces only when the version it reports equals the version the API announced**. A truncated download, an artifact for the wrong architecture, or an HTML error page saved to a file therefore cannot displace a working `ma`; any failing step cleans up the temporary file and keeps the original binary. If the binary was invoked through a symlink (for example `~/.local/bin/ma -> versions/.../ma`), the target the link points at is replaced and the link itself is left alone.

Ordinary runs also check once a day for a newer version and print one line to stderr when an update exists (not gated on `--verbose`, never written to stdout, so a piped stdout stays pure final text). The check is best-effort: any error is silently ignored, the timestamp is recorded in `~/.config/ma/last-update-check`, and it will not ask again within 24 hours. `--no-update-check` disables it entirely. Platform support and the measured matrix are in [Releasing](releasing.en.md#measured-platform-support).

## Options

| Option | Description | Default |
| --- | --- | --- |
| `--base-url URL` | Model API root | `https://api.openai.com/v1` |
| `--model MODEL` | Model name | environment or config file; required |
| `--api-key KEY` | API key; prefer the environment variable | environment or config file; required |
| `--skills NAMES` | Load skills by directory name, comma-separated | nothing loaded |
| `--write` | Allow writes inside the startup directory and its descendants | off |
| `--net` | Allow the shell to use the network | off |
| `--update` | Replace this binary with the latest release; use it on its own | — |
| `--no-update-check` | Skip the once-a-day check for a newer release | check on |
| `-v, --verbose` | Show execution live | off |
| `--max-steps N` | Maximum number of model requests | `200` |
| `--http-timeout SEC` | Per-request timeout, seconds | `120` |
| `--shell-timeout SEC` | Per-command timeout, seconds | `30` |
| `-h, --help` | Show help | — |
| `--version` | Show version; platform and runtime requirement go to stderr | — |
| `--` | Treat everything after it as the prompt | — |

## Configuration and input

Configure the model with `BASE_URL`, `MODEL` and `API_KEY`, or with the `--base-url`, `--model` and `--api-key` options (prefer the environment variable for the key). Model connection settings read only these three environment variables. The default API root is `https://api.openai.com/v1`; the client appends `/chat/completions`, so do not pass a full endpoint path.

Precedence: command line wins over environment, environment wins over the config file. The config file defaults to `~/.config/ma/config.json` (honouring `XDG_CONFIG_HOME`) and `MA_CONFIG` can point elsewhere; an explicit path that does not exist is an error. The file must be a JSON object accepting only the three string keys `base_url`, `api_key` and `model`; unknown keys, non-string values or a non-object are all errors rather than being silently ignored. **Permissions cannot be written to the config file**: `write` and `net` are rejected there, and permissions can only be granted on the command line. A config file containing `api_key` must not be readable by group or other users, otherwise startup is refused with a `chmod 600` hint.

For example, create the config file at the default location (adjust the path when using `XDG_CONFIG_HOME`):

```sh
mkdir -p ~/.config/ma
```

Save the following as `~/.config/ma/config.json`, replacing the values with your real connection settings:

```json
{
  "base_url": "https://your-endpoint.example/v1",
  "api_key": "your-key",
  "model": "your-model"
}
```

```sh
chmod 600 ~/.config/ma/config.json
```

The prompt can be one or more positional arguments; piped stdin supplies supplementary context, and stdin alone can also carry the task. Input must be UTF-8 and at most 1 MiB once combined. Use `--` when the prompt begins with `-`.

`--help` / `-h` / `--version` may appear after the prompt, for example `ma 'task' --help`, and need no valid model configuration. Values of other options are never mistaken for help flags; everything after `--` is the prompt.

```sh
printf '%s' 'list the Rust files in this directory' | ma
ma --max-steps 10 --shell-timeout 15 --http-timeout 90 'analyze why the build fails'
ma --help
```

## Streaming and execution log

By default the client sends `stream: true`, reads the Chat Completions SSE synchronously, and needs no async runtime. Model text and tool arguments are assembled from fragments, and tool calls are executed only after the complete `finish_reason` and `[DONE]` have been received; a broken stream, a malformed event or exceeding a budget exits with an error. Endpoints that return plain JSON are still supported.

By default only the complete final answer is written to stdout. `--verbose` / `-v` turns on the live log: model request rounds and durations, model text fragments, shell commands, tool stdout/stderr, exit codes, refusals, timeouts and truncation markers all go to stderr. Tool output is shown while it is drained rather than after the command ends; the two output streams are interleaved in the log but still kept separate when returned to the model. Authorization headers and API key settings are never written to the log; model and tool text in it is still the task's raw data.

```sh
ma -v 'analyze this project'
ma -v --write 'fix the config file' > answer.txt
# The final answer is saved in answer.txt while the terminal still shows the execution log on stderr
```

Verbose prints model text fragments to stderr and once more the complete final answer on stdout; a caller should consume the two streams separately. When the stderr pipe is full, optional log lines are dropped so that reading from the model and tool timeouts are not blocked; tool results returned to the model keep their original budget. The model's `reasoning_content` text is not printed, but the compatibility field and non-conflicting extension metadata are preserved for later model requests. SSE accepts LF, CRLF and CR line endings as well as a UTF-8 BOM at the start of the stream. There is no progress UI, interactive approval or extra logging framework.

## Skills

```sh
ma --skills code-review,explain 'review and explain these changes'
ma --skills=code-review 'review these changes'
```

Each name is looked up in order and the first `SKILL.md` that reads successfully is used:

1. `<startup cwd>/.agents/skills/<name>/SKILL.md`
2. `$HOME/.agents/skills/<name>/SKILL.md`

A name is a skill **directory name**, not the `name` in the frontmatter; letters, digits, hyphens and underscores are allowed, it may not start with a hyphen, and a path is not accepted. Comma-separated names are trimmed and empty entries ignored; `--skills` may be repeated, and duplicate names or skills resolving to the same real directory are loaded once, keeping the order they were selected. Without the option no skills are loaded, no other agent's private directories are scanned, and parent project directories are not searched upwards. When `HOME` is unset only the project directory is searched.

A `SKILL.md` that is missing, unreadable, non-UTF-8 or empty is skipped; a single file is at most 64 KiB and the combined raw text of all skills at most 1 MiB, and files over budget are skipped too. This is silent by default; `-v` shows what was loaded and why something was skipped, while stdout still carries only the final answer.

The full `SKILL.md` of each selected skill (frontmatter included) is added to the system prompt as a separate skills section; referenced material is not preloaded and scripts are not run. This is direct loading after an explicit choice, not a list of every skill for the model to choose from. The prompt carries both the name and the real directory; relative references resolve against that directory and the model reads them on demand through the shell using absolute paths. Capability constraints and the user's explicit task take precedence over skill instructions. Context recovery preserves the entire system prompt, so loaded skills are never removed by recovery; however the total size of the skills counts as irreducible, is never truncated automatically when it exceeds the model window, and only appears in the exit diagnostic.

A skill directory installed through a symlink is resolved to its real directory; that real directory is additionally granted read-only access, while unselected skills outside the working directory get nothing. Symlinks in files that are read are still checked against their real target, and reads escaping the working directory and the selected skill directories are refused; a `SKILL.md` pointing outside its skill directory is skipped. Write permission is still limited to the startup cwd and its descendants, and network access is still controlled by `--net`; frontmatter such as `allowed-tools` changes no permissions and registers no new tools. External references can use the existing read-only commands such as `cat` and `head`; script execution remains subject to the current Guard, and compatibility with skills written for other agents or with complex scripts is not guaranteed.

Only name input is supported today; a custom single skill directory or a directory holding several skills is left for later.

## Context too long recovery

A normal run only appends messages and never trims on its own. Recovery is triggered only when upstream explicitly reports a context-length error: a bounded JSON error body on HTTP 400/413/422, or an `error` in a JSON/SSE response. It recognises the `context_length_exceeded` / `context_window_exceeded` code or type, and messages that explicitly describe the context length or window being exceeded; ordinary 400s, authentication failures, rate limits, server errors and timeouts do not trigger it.

Recovery is a three-tier ladder, tried in order of how much information it costs; a tier is not used while the previous one can still help.

1. **Compact old turns.** Every tool call except those in the newest turn is compacted into a one-line skeleton: the command (truncated for display), exit code, stdout/stderr byte counts, and timeout and truncation markers. All of it comes from tool results that were already parsed -- no model call, no tokenizer, and repeatable. The skeleton is written into a single summary slot at index 2 that **later recoveries rebuild rather than append to**, so its total length is capped at 64 KiB and the oldest entries are dropped when it overflows. This tier deletes no turn and re-runs no command.
2. **Truncate the newest turn's tool results.** Each result keeps its first 2 KiB and last 2 KiB, with the number of omitted bytes marked in between. `exit_code`, `timed_out`, `truncated` and the `tool_call_id` pairing are all preserved; the assistant message carrying `tool_calls` is left untouched, so no dangling call can appear.
3. **Truncate the original input.** messages[1] keeps its first 32 KiB and last 32 KiB. The system prompt and loaded skills are never rewritten.

Each tier leaves a recognisable marker in the history, which is how the ladder tells whether that tier still needs to run; repeated context errors therefore never trim the same bytes twice and never pile up more summary slots.

**Loss boundary (contract).** Tier 1 loses no commands and no exit status, only output bodies. Output lost at tier 2 is **not recoverable**, because the process that produced it has exited. Tier 3 is **data destruction**: piped input exists only in messages[1], never landed on disk, and the omitted middle cannot be read again -- only the command that produced it can be re-run. Tier 3 prints a warning to stderr **unconditionally** (no `--verbose` needed), and the summary and note also ask the model to say so in its final answer when the omitted content matters. Workspace files are not in this category: the model can re-read them at any time.

Each context failure applies one tier and retries the model once; the retry counts towards `--max-steps` and processed shell commands are never re-run. A new context error continues from the tier that just ran (using it again if it is still effective) until all three are exhausted. When none of the three can change the history any further, the program exits with an error and reports the byte counts of the system prompt, the original input, the newest turn, the skills and the total -- that is, every part recovery cannot reach. `--verbose` logs the concrete action of each tier.

For example, with `git diff | ma 'summarize these changes'`: if the diff exceeds the model window, earlier tool turns are compacted first (usually there are none here), then the newest results are truncated, and finally the diff itself is truncated with a warning. The previous delete-only recovery could free only single-digit percentages on this shape, or fail outright when there was no turn it was allowed to delete.

This introduces no model summarisation, tokenizer, proactive context budget or extra configuration. Compaction loses output bodies and truncation is not guaranteed to be lossless; either may also reduce cache reuse.

## Permissions

Permissions are granted when the process starts, are never asked for or upgraded while it runs, and a refusal is reported to the model as a tool result.

| How it was started | Reading workspace data | Write scope | Shell network |
| --- | --- | --- | --- |
| default | allowed | refused | refused |
| `--write` | allowed | startup cwd and its descendants | refused |
| `--net` | allowed | refused | allowed |
| `--write --net` | allowed | startup cwd and its descendants | allowed |

Model API requests are always allowed and are separate from the shell's `--net`. System executables and the system resources they need remain available. Skill directories selected with `--skills` are additionally readable; other extra authorization directories are not supported.

One Policy produces the model instruction and drives the Guard checks. The Guard blocks common write commands, file redirections and obvious network commands, and checks literal paths for parent directories, existing symlinks and common output options. `/dev/null` is a usable output target and descriptor redirections such as `>&2` are allowed.

**The Guard is a best-effort safety gate, not a security sandbox.** Unknown CLIs, writes or network access inside a program, build scripts, Git hooks, configuration effects and path changes after the check can all produce side effects it does not recognise; these capability flags provide no OS-enforced isolation. Run model-generated commands in an environment you trust, and leave strong isolation to a future OS sandbox.

Read-only file commands (`cat head tail ls stat wc du file readlink`) support globs inside the working directory, for example `cat *.rs`, `wc -l *.rs`, `ls ./*.rs` and `cat src/*/*.rs`. Before execution the Guard checks both the literal path (which the shell uses when nothing matches) and the candidates matched level by level, refusing directories and symlinks that escape; each argument is checked against at most 16384 directory entries, and beyond that it asks for a narrower path. `*`, `?` and plain character ranges are supported; a character range mixed with quoting or escaping is conservatively refused with a hint to use a literal path or `*` / `?`. A wildcard inside quotes keeps its literal meaning. The Guard matches characters according to the host locale, as `bash` and busybox `ash` do; on Debian and Ubuntu `/bin/sh` is `dash`, which matches `?` bytewise, so `?.rs` there does not match UTF-8 file names (such as `你.rs`), and the Guard conservatively does not expand those candidates either.

To keep the Guard simple, V0 still refuses variable expansion, command substitution, nested shells, control flow, environment prefixes, privilege escalation, `ln`, and command forwarding such as `xargs` / `find -exec`. `{}` is accepted as an ordinary literal; a refusal of `find -exec cat {} +` names `find -exec` explicitly, while real shell grouping is still refused. `cd` is not mixed with pipelines, background branches or `||`; `CDPATH` is cleared so the environment cannot change the effective working directory. Write paths must be literal and globs are not supported as write targets. Downloads write to an explicit output path; implicit file names such as `curl -O` and composite write options that cannot be checked are refused. Complex commands can be split into several shell calls, and single quotes can be used for text patterns and script arguments. Package managers and build tools are conservatively treated as writes; supported offline flags (for example `cargo --offline`, `mvn -o`) waive the obvious network check. Other complex syntax may be refused by mistake, and the model adapts its command.

## Invocation contract

- stdout: the complete final text answer only, with a trailing newline; streaming intermediate text is never written to this channel.
- stderr: argument, input, HTTP, protocol and loop-termination errors; with `--verbose` it additionally carries live model text and the execution log.
- Exit codes: `0` final answer produced; `1` runtime/HTTP/protocol error; `2` argument/input error; `3` step limit exhausted. `0` means a final was obtained, not that the task the model claimed to complete succeeded.
- `--max-steps` defaults to `200` and counts model requests. The last step still requests the model but does not execute tool calls whose results could no longer be fed back.
- `--http-timeout` defaults to `120` seconds and `--shell-timeout` to `30` seconds; both are positive integers up to `86400`.
- Each response (including a complete SSE stream) is at most 4 MiB and at most 64 tool calls; tools run in order. The HTTP timeout limits the whole response time, streaming reads included.
- Shell stdin is closed, so interactive commands are not supported. Every call runs from the startup working directory and `cd` does not persist across tool calls.
- stdout and stderr each keep the first 64 KiB; the rest is still drained but not stored, and the result carries `truncated`.
- The Unix shell runs in its own process group; a timeout kills that group, and remaining group members are cleaned up when the shell exits. On SIGINT, SIGTERM or SIGHUP the agent kills the current tool process group and exits with `128 + signal`. SIGKILL and programs that deliberately leave the process group are outside what this cleanup guarantees.
- `API_KEY` (and the legacy `MA_API_KEY` / `OPENAI_API_KEY` a host may still carry) is not passed to shell children; the rest of the host environment is still inherited.
- Ordinary HTTP errors are not retried automatically; only the explicit context-too-long error above can recover along the ladder and retry once. Redirects are not followed; TLS validation uses the root certificates built into the dependency and certificate verification is never disabled.

A tool result is a JSON string containing `stdout`, `stderr`, `exit_code`, `timed_out` and `truncated`; a refusal or execution error additionally carries `error`. The model protocol follows [Chat Completions function calling](https://developers.openai.com/api/docs/guides/function-calling). Assistant messages and tool IDs are preserved; an empty final, a truncated completion or a malformed protocol exits with an error.

## Verification

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
cargo build --release --locked
wc -c target/release/ma
```

The tests use temporary working directories, a real `/bin/sh` and a local mock HTTP server; no key or real model is needed. CI runs the same checks on macOS and Linux and requires the release binary to be under 10,000,000 bytes; the target is under 5,000,000 bytes. Tools such as `rg`, `fd` and `jq` are optional host utilities, not runtime dependencies of `ma`.

The two scripts in this repository:

```sh
./scripts/install.sh    # install into ~/.local/bin
./scripts/release.sh    # release: bump -> verify -> commit -> tag -> push
```

Usage of `release.sh` and its requirements on the working tree are in [Releasing](releasing.en.md#scripted-scriptsreleasesh).
