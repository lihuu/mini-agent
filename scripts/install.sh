#!/bin/sh
# Build and install `ma` into a user-level bin directory.
#
# Defaults to ~/.local/bin, which is the XDG user bin directory and normally
# precedes ~/.cargo/bin on PATH. Installing there means a later `ma` dropped
# into ~/.local/bin cannot end up silently shadowed by a stale cargo copy.
set -eu

usage() {
    cat <<'EOF'
Usage: scripts/install.sh [OPTIONS]

  --prefix DIR    Install root; binary goes to DIR/bin (default: ~/.local)
  --offline       Pass --offline to cargo (no network access)
  --uninstall     Remove the installed binary and exit
  -h, --help      Show this help

Environment: CARGO, PREFIX
EOF
}

root="${PREFIX:-$HOME/.local}"
offline=""
uninstall=""
cargo_bin="${CARGO:-cargo}"

while [ $# -gt 0 ]; do
    case "$1" in
        --prefix)
            [ $# -ge 2 ] || { echo "install.sh: --prefix requires a value" >&2; exit 2; }
            root="$2"
            shift 2
            ;;
        --prefix=*)
            root="${1#--prefix=}"
            shift
            ;;
        --offline)
            offline="--offline"
            shift
            ;;
        --uninstall)
            uninstall="1"
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "install.sh: unknown option: $1" >&2
            usage >&2
            exit 2
            ;;
    esac
done

# Resolve the project root from this script's location so the script works
# from any working directory, and through a symlinked checkout.
# Clear CDPATH first: a CDPATH entry would otherwise make `cd` print the path
# instead of chdir'ing there, corrupting the resolved location.
script_dir=$(unset CDPATH; cd -- "$(dirname -- "$0")" && pwd)
project=$(unset CDPATH; cd -- "$script_dir/.." && pwd)
[ -f "$project/Cargo.toml" ] || {
    echo "install.sh: no Cargo.toml in $project" >&2
    exit 1
}

target="$root/bin/ma"

if [ -n "$uninstall" ]; then
    if [ -e "$target" ] || [ -L "$target" ]; then
        rm -f -- "$target"
        echo "removed $target"
    else
        echo "nothing to remove at $target"
    fi
    exit 0
fi

command -v "$cargo_bin" >/dev/null 2>&1 || {
    echo "install.sh: cargo not found; install Rust 1.85+ first" >&2
    exit 1
}

echo "installing into $root/bin"
"$cargo_bin" install --path "$project" --locked --force --root "$root" $offline

# A previous default-root install stays on PATH and can mask or confuse the new
# one. Remove it only when cargo owns it; never touch a foreign binary.
cargo_home="${CARGO_HOME:-$HOME/.cargo}"
stale="$cargo_home/bin/ma"
installed_dir=$(unset CDPATH; cd -- "$root" && pwd)
stale_dir=$(unset CDPATH; cd -- "$cargo_home" && pwd)
if [ "$installed_dir" != "$stale_dir" ] && [ -e "$stale" ]; then
    if "$cargo_bin" install --list 2>/dev/null | grep -qE '^(oneagent|mini-agent) '; then
        "$cargo_bin" uninstall oneagent >/dev/null 2>&1 \
            || "$cargo_bin" uninstall mini-agent >/dev/null 2>&1 || true
        echo "removed the older copy at $stale"
    else
        echo "note: $stale exists but is not cargo-managed; left untouched" >&2
    fi
fi

[ -x "$target" ] || {
    echo "install.sh: expected $target after install" >&2
    exit 1
}
echo "installed $("$target" --version)"

case ":$PATH:" in
    *":$root/bin:"*) ;;
    *) echo "note: add $root/bin to PATH (it is not there yet)" >&2 ;;
esac
