#!/bin/bash
# Shared helpers for the build/release scripts. Source this, don't execute it.
#   . "$ROOT/scripts/lib.sh"

# The canonical app version: the [package] version in Cargo.toml. This is the
# single source of truth so a bundle never ships a stale hardcoded number.
cargo_version() {
    local manifest="${1:-Cargo.toml}"
    # First `version = "x.y.z"` line — [package] is the first table in the file.
    sed -n 's/^version = "\(.*\)"/\1/p' "$manifest" | head -1
}

# Abort with a clear message if any required tool is missing from PATH.
require() {
    local tool missing=0
    for tool in "$@"; do
        if ! command -v "$tool" >/dev/null 2>&1; then
            echo "ERROR: required tool '$tool' not found in PATH" >&2
            missing=1
        fi
    done
    [ "$missing" -eq 0 ] || exit 1
}
