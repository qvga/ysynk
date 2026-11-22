#!/bin/sh
# Run in a prepared Linux checkout (see CONTRIBUTING.md).
set -eu
cd "$(dirname "$0")/.."
cargo fmt --all --check
npm run check
npm test
npm run test:dependencies
npm run build
python3 -m unittest discover -s tools -p 'test_*.py'
python3 tools/check-backports.py
python3 tools/release-notices.py
cargo test --workspace --locked
cargo clippy --workspace --locked --all-targets -- -D warnings
