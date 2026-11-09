#!/bin/sh
# Compatibility entry point: build both light presentations with the shared builder.
set -eu
script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
exec "$script_dir/build-light-simulators.sh"
