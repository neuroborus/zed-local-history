#!/usr/bin/env sh
set -eu

APP_NAME="local-history"
BINARIES="local-history local-history-sidecar local-history-mcp"

usage() {
    cat <<'EOF'
Usage: ./install.sh [--prefix DIR] [--bin-dir DIR] [--dry-run]

Install local-history release binaries from this extracted bundle.

Options:
  --prefix DIR   Install into DIR/bin. Defaults to $PREFIX or $HOME/.local.
  --bin-dir DIR  Install directly into DIR. Overrides --prefix.
  --dry-run      Print the install actions without changing files.
  -h, --help     Show this help.
EOF
}

die() {
    printf '%s: %s\n' "$APP_NAME" "$*" >&2
    exit 1
}

run() {
    if [ "$DRY_RUN" -eq 1 ]; then
        printf '+'
        for arg in "$@"; do
            printf ' %s' "$arg"
        done
        printf '\n'
    else
        "$@"
    fi
}

PREFIX="${PREFIX:-}"
BIN_DIR=""
DRY_RUN=0

while [ "$#" -gt 0 ]; do
    case "$1" in
        --prefix)
            [ "$#" -ge 2 ] || die "--prefix requires a directory"
            PREFIX="$2"
            shift 2
            ;;
        --prefix=*)
            PREFIX="${1#--prefix=}"
            shift
            ;;
        --bin-dir)
            [ "$#" -ge 2 ] || die "--bin-dir requires a directory"
            BIN_DIR="$2"
            shift 2
            ;;
        --bin-dir=*)
            BIN_DIR="${1#--bin-dir=}"
            shift
            ;;
        --dry-run)
            DRY_RUN=1
            shift
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            die "unknown option: $1"
            ;;
    esac
done

if [ -z "$BIN_DIR" ]; then
    if [ -z "$PREFIX" ]; then
        [ -n "${HOME:-}" ] || die "HOME is not set; pass --prefix or --bin-dir"
        PREFIX="$HOME/.local"
    fi
    BIN_DIR="$PREFIX/bin"
fi

SCRIPT_DIR=$(CDPATH= cd "$(dirname "$0")" && pwd)

for binary in $BINARIES; do
    [ -f "$SCRIPT_DIR/$binary" ] || die "missing $binary next to install.sh; run this script from an extracted release bundle"
done

if command -v install >/dev/null 2>&1; then
    run install -d "$BIN_DIR"
    for binary in $BINARIES; do
        run install -m 0755 "$SCRIPT_DIR/$binary" "$BIN_DIR/$binary"
    done
else
    run mkdir -p "$BIN_DIR"
    for binary in $BINARIES; do
        run cp "$SCRIPT_DIR/$binary" "$BIN_DIR/$binary"
        run chmod 0755 "$BIN_DIR/$binary"
    done
fi

printf '\nInstalled %s binaries to %s\n' "$APP_NAME" "$BIN_DIR"

case ":${PATH:-}:" in
    *:"$BIN_DIR":*)
        printf 'Verify with:\n  local-history --help\n'
        ;;
    *)
        printf 'Add this directory to PATH before running local-history:\n'
        printf '  export PATH="%s:$PATH"\n' "$BIN_DIR"
        printf 'Then verify with:\n  local-history --help\n'
        ;;
esac
