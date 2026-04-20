#!/usr/bin/env sh
set -eu

PREFIX="${PREFIX:-$HOME/.local}"
BIN_DIR="$PREFIX/bin"
REPO_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
CLI_PATH="$REPO_DIR/crates/themion-cli"
PACKAGE_VERSION=""
CARGO_ARGS=""
FEATURES=""
ALL_FEATURES=0

need_cmd() {
    if ! command -v "$1" >/dev/null 2>&1; then
        echo "error: required command not found: $1" >&2
        exit 1
    fi
}

usage() {
    cat <<EOF
Usage: ./install.sh [options] [-- <extra cargo install args>]

Install themion from this repository using cargo.

Options:
  --help              Show this help
  --prefix <dir>      Install root prefix (default: ~/.local or \$PREFIX)
  --package-version <version>
                      Install a specific package version with cargo install --version
  --locked            Pass --locked to cargo install
  --features <list>   Pass --features <list> to cargo install
  --all-features      Enable all crate features during install

Examples:
  ./install.sh
  ./install.sh --prefix "$HOME/.cargo"
  ./install.sh --locked
  ./install.sh --features stylos
  ./install.sh --all-features
  ./install.sh --package-version 0.11.0
  ./install.sh -- --force
EOF
}

append_arg() {
    if [ -z "$CARGO_ARGS" ]; then
        CARGO_ARGS="$1"
    else
        CARGO_ARGS="$CARGO_ARGS $1"
    fi
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --help|-h)
            usage
            exit 0
            ;;
        --prefix)
            [ "$#" -ge 2 ] || { echo "error: --prefix requires a value" >&2; exit 1; }
            PREFIX="$2"
            BIN_DIR="$PREFIX/bin"
            shift 2
            ;;
        --package-version)
            [ "$#" -ge 2 ] || { echo "error: --package-version requires a value" >&2; exit 1; }
            PACKAGE_VERSION="$2"
            shift 2
            ;;
        --locked)
            append_arg "--locked"
            shift
            ;;
        --features)
            [ "$#" -ge 2 ] || { echo "error: --features requires a value" >&2; exit 1; }
            FEATURES="$2"
            shift 2
            ;;
        --all-features)
            ALL_FEATURES=1
            shift
            ;;
        --)
            shift
            while [ "$#" -gt 0 ]; do
                append_arg "$1"
                shift
            done
            ;;
        *)
            echo "error: unknown option: $1" >&2
            echo "try ./install.sh --help" >&2
            exit 1
            ;;
    esac
done

if [ -n "$FEATURES" ] && [ "$ALL_FEATURES" -eq 1 ]; then
    echo "error: --features and --all-features cannot be used together" >&2
    exit 1
fi

need_cmd cargo

if [ ! -f "$CLI_PATH/Cargo.toml" ]; then
    echo "error: could not find themion CLI crate at $CLI_PATH" >&2
    exit 1
fi

mkdir -p "$BIN_DIR"

echo "Installing themion to $PREFIX"
echo "Source: $REPO_DIR"

set -- install --path "$CLI_PATH" --root "$PREFIX"

if [ -n "$PACKAGE_VERSION" ]; then
    set -- "$@" --version "$PACKAGE_VERSION"
fi

if [ -n "$FEATURES" ]; then
    set -- "$@" --features "$FEATURES"
fi

if [ "$ALL_FEATURES" -eq 1 ]; then
    set -- "$@" --all-features
fi

if [ -n "$CARGO_ARGS" ]; then
    # shellcheck disable=SC2086
    set -- "$@" $CARGO_ARGS
fi

cargo "$@"

TARGET_BIN="$BIN_DIR/themion"
if [ -x "$TARGET_BIN" ]; then
    echo "Installed: $TARGET_BIN"
else
    echo "warning: cargo install finished but $TARGET_BIN was not found" >&2
fi

case ":${PATH:-}:" in
    *:"$BIN_DIR":*) ;;
    *)
        echo
        echo "Note: $BIN_DIR is not currently on your PATH."
        echo "Add this to your shell config if needed:"
        echo "  export PATH=\"$BIN_DIR:\$PATH\""
        ;;
esac
