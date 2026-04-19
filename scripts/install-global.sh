#!/usr/bin/env sh
set -eu

repo_dir=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)

echo "Installing hmm globally from: $repo_dir"
cargo install --path "$repo_dir" --locked --force

cat <<'EOF'

If `hmm` is not found yet, add Cargo bin to your PATH:

  export PATH="$HOME/.cargo/bin:$PATH"

EOF
