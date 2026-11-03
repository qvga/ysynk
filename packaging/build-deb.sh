#!/bin/sh
# Build only. Never install packages, enable startup or modify repositories.
set -eu
cd "$(dirname "$0")/.."
python3 tools/release-notices.py
npm run desktop:build -- --bundles deb
printf '\nDebian package output: target/release/bundle/deb/\n'
