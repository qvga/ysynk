#!/bin/sh
# Build the bundled UI and a cached debug binary, then replace this process.
# No development server, service, installation or repository initialization.
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
cd "$root"

for tool in npm cargo python3; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        printf 'Missing %s. Install the project prerequisites first.\n' "$tool" >&2
        exit 1
    fi
done
if [ ! -d node_modules ]; then
    printf 'Frontend dependencies are missing. Run npm ci first.\n' >&2
    exit 1
fi

target_dir=${CARGO_TARGET_DIR:-"$root/target"}
case "$target_dir" in
    /*) ;;
    *) target_dir="$root/$target_dir" ;;
esac
export CARGO_TARGET_DIR="$target_dir"

# The native bundle references this generated, offline notice inventory.
python3 tools/release-notices.py
npm run desktop:build -- --debug --no-bundle

exec "$target_dir/debug/ysynk-desktop" "$@"
