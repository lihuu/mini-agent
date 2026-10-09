#!/bin/sh
# Cut a release: bump the version, verify, commit, tag and push. Pushing the tag
# is what triggers .github/workflows/release.yml to build and publish the
# binaries, so this script ends by handing off to CI.
#
# The tag name is checked against Cargo.toml by the workflow's verify job; that
# check is the reason the version bump here is not optional.
set -eu

usage() {
    cat <<'EOF'
Usage: scripts/release.sh [OPTIONS] <major|minor|patch|x.y.z>

  --dry-run    Do every check and print the plan, but do not write or push.
  --no-push    Commit and tag locally, then stop before pushing.
  --offline    Pass --offline to cargo (no network access).
  -h, --help   Show this help.

The version argument is either a new version ("0.4.0") or the part to
increment. The release is published by the workflow triggered by the tag,
not by this script.
EOF
}

dry_run=""
push="1"
offline=""
cargo_bin="${CARGO:-cargo}"

while [ $# -gt 0 ]; do
    case "$1" in
        --dry-run) dry_run="1"; shift ;;
        --no-push) push=""; shift ;;
        --offline) offline="--offline"; shift ;;
        -h|--help) usage; exit 0 ;;
        -*) echo "release.sh: unknown option: $1" >&2; usage >&2; exit 2 ;;
        *) break ;;
    esac
done

[ $# -eq 1 ] || { usage >&2; exit 2; }
bump="$1"

# Resolve the project root from this script's location so it runs from anywhere.
script_dir=$(unset CDPATH; cd -- "$(dirname -- "$0")" && pwd)
project=$(unset CDPATH; cd -- "$script_dir/.." && pwd)
cd -- "$project"
[ -f Cargo.toml ] || { echo "release.sh: no Cargo.toml in $project" >&2; exit 1; }

run() {
    if [ -n "$dry_run" ]; then
        echo "  would run: $*"
    else
        "$@"
    fi
}

# --- preconditions ---------------------------------------------------------

branch=$(git rev-parse --abbrev-ref HEAD)
[ "$branch" = "main" ] || {
    echo "release.sh: on branch '$branch'; releases are cut from main" >&2
    exit 1
}
if [ -n "$(git status --porcelain)" ]; then
    echo "release.sh: working tree is dirty; commit or stash first" >&2
    exit 1
fi
command -v "$cargo_bin" >/dev/null 2>&1 || {
    echo "release.sh: cargo not found" >&2
    exit 1
}

# --- work out the new version ----------------------------------------------

current=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
[ -n "$current" ] || { echo "release.sh: no version in Cargo.toml" >&2; exit 1; }

# Split x.y.z, rejecting anything else so a typo cannot create a strange tag.
major=${current%%.*}
rest=${current#*.}
minor=${rest%%.*}
patch=${rest#*.}
case "$major$minor$patch" in
    *[!0-9]*|"") echo "release.sh: current version '$current' is not x.y.z" >&2; exit 1 ;;
esac

case "$bump" in
    major) next="$((major + 1)).0.0" ;;
    minor) next="$major.$((minor + 1)).0" ;;
    patch) next="$major.$minor.$((patch + 1))" ;;
    *)
        next="${bump#v}"
        echo "$next" | grep -qE '^[0-9]+\.[0-9]+\.[0-9]+$' || {
            echo "release.sh: '$bump' is neither major/minor/patch nor x.y.z" >&2
            exit 2
        }
        ;;
esac

if git rev-parse -q --verify "refs/tags/v$next" >/dev/null; then
    echo "release.sh: tag v$next already exists" >&2
    exit 1
fi

echo "release: $current -> $next"
[ -n "$dry_run" ] || echo "  (dry run: nothing will be written)"

# --- verify, bump, commit, tag, push ---------------------------------------

echo "verifying $current before the bump"
for check in "fmt --check" "clippy --locked --all-targets -- -D warnings" "test --locked --release"; do
    # shellcheck disable=SC2086
    run "$cargo_bin" $check $offline
done

if [ -n "$dry_run" ]; then
    echo "  would write version = \"$next\" to Cargo.toml"
else
    sed -i.bak "s/^version = \"$current\"/version = \"$next\"/" Cargo.toml
    rm -f Cargo.toml.bak
    grep -q "^version = \"$next\"$" Cargo.toml || {
        echo "release.sh: failed to rewrite Cargo.toml" >&2
        exit 1
    }
    run "$cargo_bin" update --workspace $offline
fi

run git add Cargo.toml Cargo.lock
run git commit -m "Release v$next"
run git tag -a "v$next" -m "v$next"
echo "release: committed and tagged v$next"

if [ -z "$push" ]; then
    echo "release: --no-push given; run 'git push origin main && git push origin v$next' to publish"
    exit 0
fi
if [ -n "$dry_run" ]; then
    echo "  would run: git push origin main"
    echo "  would run: git push origin v$next"
    echo "dry run complete; the workflow would then build and publish v$next"
    exit 0
fi

git push origin main
git push origin "v$next"
echo "release: pushed v$next; CI is building the binaries"
echo "  https://github.com/lihuu/oneagent/actions"
