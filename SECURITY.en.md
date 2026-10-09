# Security Policy

[中文](SECURITY.md) · English

## `ma` executes model-generated shell commands

`ma` hands model-generated commands to `/bin/sh -c`. The built-in Guard only performs **best-effort checks fixed at startup**, meant to catch common mistakes:

- It is **not a security sandbox** and provides no OS-enforced isolation of any kind.
- Unknown CLIs, writes or network access inside a program, build scripts, Git hooks, interpreters (`python`/`node`/`perl`/`ruby`), redirections inside quotes, and path changes after the check can all get around the Guard.
- `--write` only means "writes inside the startup cwd are intended to be allowed"; it does not mean out-of-bounds writes are reliably prevented.
- `--net` only blocks obvious network commands by name; model API requests are always allowed.
- Skill instructions explicitly selected with `--skills` are added to the system prompt, and the real skill directories are additionally granted read-only access; this widens no write or network permission. Select only skills you trust, and note that their instructions and scripts are still subject to the Guard limitations above.

**When using `ma` in an untrusted environment, put it in a container or virtual machine with a read-only mount and a network policy for real isolation.** Do not point it at content that prompt injection can influence and still expect the Guard to protect the host.

## Supported versions

Only the latest commit on `main` is maintained.

## Reporting a vulnerability

Please use GitHub's private vulnerability reporting: on the repository page go to **Security** -> **Report a vulnerability**. Please do not disclose an unfixed bypass in a public issue.

Where possible, include:

- the `ma` version/commit and the platform it runs on;
- the full `shell(command)` argument or the prompt that triggers it;
- the startup options (`--write` / `--net`);
- the out-of-bounds write or network access that actually happened, and the behaviour you expected.

## Non-goals

The following are not security vulnerabilities and should not be reported:

- the Guard not blocking some CLI that is not on its list (a known best-effort boundary, see the README and the section above);
- the model running a destructive command under `--write --net` (that is what the permission design allows);
- the model reaching wrong conclusions or claiming success (the contract only guarantees that a `final` was obtained, not that the task succeeded).
