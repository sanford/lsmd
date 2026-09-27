#!/usr/bin/env bash
# Build lsmd, install it to ~/.local/bin, and run it.
# Any arguments are passed through: ./run.sh README.md, ./run.sh -p x.md, ...
set -euo pipefail

cd "$(dirname "$0")"
bin_dir="$HOME/.local/bin"

cargo build --release --quiet
mkdir -p "$bin_dir"
install -m 755 target/release/lsmd "$bin_dir/lsmd"

exec "$bin_dir/lsmd" "$@"
