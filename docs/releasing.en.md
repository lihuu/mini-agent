# Releasing

A release is triggered by a tag; `.github/workflows/release.yml` performs the verification, builds and publication automatically. A human does three things: bump the version, commit, and tag and push.

## Steps

```sh
# 1. Bump the version (the single source of truth is Cargo.toml)
$EDITOR Cargo.toml              # version = "0.3.4"
cargo build --offline           # sync the mini-agent version in Cargo.lock

# 2. Commit (separate from feature commits, to keep the history readable)
git add Cargo.toml Cargo.lock
git commit -m "Release v0.3.4"

# 3. Annotated tag + push
git tag -a v0.3.4 -m "v0.3.4 — <one line on what this release changes>"
git push origin main
git push origin v0.3.4
```

Pushing the tag starts the workflow and it finishes in about two minutes.

## Always verify locally before pushing

CI only runs `verify` on ubuntu, so **some problems are invisible locally on macOS**. The fixed routine before a release:

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --release
```

A past lesson: the v0.3.2 tag failed `verify` after being pushed, because a unit test hardcoded the release artifact name as `aarch64-apple-darwin` while `release_target()` returns `x86_64-unknown-linux-gnu` on the Linux CI runner. **Anything platform-dependent must be derived from `release_target()`, never written out by hand.** That failure also showed the gate works: it stopped the build, and no artifact was produced at all.

## What the workflow does

| Job | Contents |
| --- | --- |
| `verify` | ubuntu-latest: fmt, clippy (`-D warnings`), `cargo test --release`, and a check that the **tag matches the `Cargo.toml` version** |
| `build` | Matrix build on macos-latest (`aarch64-apple-darwin`) and ubuntu-latest (`x86_64-unknown-linux-gnu`), each packed into `ma-<version>-<target>.tar.gz` containing `ma`, `LICENSE`, `README.md` and `SECURITY.md`, with a check that the binary is under 10 MB |
| `release` | Collects the artifacts and publishes them directly with `softprops/action-gh-release`, generating the changelog |

If `verify` fails, `build` does not run (`needs: verify`), so a wrong version number or a failing test never ships an artifact.

Publication is **direct** (not a draft). Quality is backstopped by `verify` rather than by a human clicking publish.

### The tag must match Cargo.toml

This step in `verify` is deliberate:

```sh
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
test "v$version" = "$GITHUB_REF_NAME"
```

It turns "remember to bump the version" from a human habit into a machine check. Otherwise you get a tag reading `v0.4.0` while the binary calls itself `0.3.1`.

### A tag trigger reads the commit that was tagged

With `on: push: tags`, GitHub reads the workflow file **in the commit that was tagged**, not the latest one on the branch. Tagging an older commit that predates `release.yml` therefore does not trigger a build, and re-pushing the tag does not help either (short of force-rewriting the tag, which you should not do). **The only way is to release a new version.**

## How users update

Three routes, same artifacts:

```sh
./scripts/install.sh        # build from source into ~/.local/bin
ma --update                 # an installed ma updates itself to the latest release
# or download the tarball from the Releases page
```

The reliability of `ma --update` comes from verifying with the artifact itself: after downloading, it runs the candidate binary with `--version` and replaces the current one only when the reported version matches the one the API announced; any failing step cleans up the temporary file and keeps the original binary. See [Usage](usage.en.md#updating).

## Measured platform support

### macOS

The artifact targets `minos 11.0` (Big Sur and later) and links only system libraries under `/usr/lib` (`libSystem`, `libiconv`). macOS does not use symbol versioning, so **there is no equivalent of the Linux glibc wall**: a supported system version is enough to run it.

### Linux

The artifact is `x86_64-unknown-linux-gnu` (dynamically linked against glibc) and was measured in the following environments:

| System | glibc | Result |
| --- | --- | --- |
| Ubuntu 24.04 | 2.39 | runs |
| Debian 12 | 2.36 | runs |
| Ubuntu 22.04 | 2.35 | runs |
| Ubuntu 20.04 | 2.31 | fails |
| Debian 11 | 2.31 | fails |
| Alpine | musl | fails (not glibc) |

**The floor is glibc 2.34.** Ubuntu 22.04 and newer glibc distributions work; 20.04, Debian 11 and older do not.

About the higher glibc on the build host (CI currently uses ubuntu-latest, i.e. 24.04):

- The artifact does reference `GLIBC_2.39` symbols (`pidfd_spawnp`, `pidfd_getpid`, the process-creation interface added in glibc 2.39), but they are recorded as **WEAK**; ld.so binds them to NULL when they are missing and does not abort loading.
- What actually makes 20.04 fail is the strong requirement on `GLIBC_2.32` / `2.33` / `2.34`, which has nothing to do with 2.39.
- As a result, systems on 2.35/2.36 print one line, `weak version GLIBC_2.39 not found`, but **work normally**. That line comes from ld.so and cannot be suppressed by the program itself.

Lowering the glibc floor (for instance by building on `ubuntu-22.04`) is not planned for now. If Ubuntu 20.04 / Debian 11 support is ever needed, change `ubuntu-latest` to `ubuntu-22.04` in the `build` matrix and the floor moves down with it.

### `--version` is a machine contract; do not annotate it

The **stdout of `--version` must be exactly one line, `ma <version>`** -- no parentheses, no suffix, no second line.

The upgrade path depends on it: `ma --update` uses the candidate binary's `--version` output for an exact equality check, and **already-published older versions compare the whole line**. Grow that line even slightly and every older version's `ma --update` will permanently refuse the new release -- those users can never upgrade again.

The platform information therefore goes to **stderr**, consistent with the rest of the project (stdout for machines, stderr for people):

```
$ ma --version
ma 0.3.4                                                <- stdout, read by machines
ma: built for aarch64-apple-darwin, requires macOS 11+  <- stderr, read by people
```

`ma --update` reads only stdout, so the two never interfere. Before touching `--version`, look at the test `version_line_is_exactly_the_bare_form_older_binaries_compare_against`, which pins this constraint down.

### Platforms without an artifact

`ma --update` relies on the `release_target()` mapping, which today covers only macOS (arm64 / x86_64) and Linux (x86_64 / aarch64). Other platforms get a clear "no prebuilt binary is published for this platform" instead of downloading an artifact for the wrong architecture.

## Packaging locally (without CI)

`dist/` is ignored by `.gitignore`. To reproduce the CI packaging by hand:

```sh
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
triple=$(rustc -vV | sed -n 's/^host: //p')
name="ma-$version-$triple"
mkdir -p "dist/$name"
cp target/release/ma LICENSE README.md SECURITY.md "dist/$name/"
chmod +x "dist/$name/ma"
(cd dist && tar -czf "$name.tar.gz" "$name")
```

Both path shapes inside the archive (`ma` or `<directory>/ma`) are accepted by the extraction logic in `ma --update`.

## Scripted: `scripts/release.sh`

The manual steps above are captured in a script:

```sh
./scripts/release.sh patch             # 0.3.6 -> 0.3.7
./scripts/release.sh minor
./scripts/release.sh major
./scripts/release.sh 1.0.0             # or give the version directly
./scripts/release.sh --dry-run patch   # print the plan, write nothing
./scripts/release.sh --no-push patch   # commit and tag, but do not push
```

The script does the following in order and stops at the first failure:

1. Refuses to start on a branch other than `main` or on a dirty working tree
2. Refuses to reuse an existing tag
3. Runs `cargo fmt --check`, `clippy --locked -D warnings` and `test --locked --release` in turn
4. Rewrites `Cargo.toml` and syncs `Cargo.lock` with `cargo update --workspace`
5. Commits as `Release vX.Y.Z` and creates an annotated tag
6. Pushes `main` and the tag, handing the build over to the workflow

**It stops at the push; the artifacts come from CI.** A local failure therefore never leaves half a version behind.
