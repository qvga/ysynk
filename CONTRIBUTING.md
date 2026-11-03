# Development

## Prerequisites

Use the Rust and Node.js versions pinned in `rust-toolchain.toml` and `.nvmrc`.
On Linux, install a C compiler, pkg-config, GTK3, WebKitGTK 4.1, AppIndicator
and D-Bus development libraries. Running backups also requires restic 0.17.1
or newer and a persistent Secret Service keychain.
The checks use Python 3; Unix detached-process regressions invoke
`/usr/bin/python3` inside disposable fixtures.

```sh
npm ci
./dev.sh
```

For frontend hot reload, start `npm run dev` yourself and run `npm run desktop`
in a second terminal. `YSYNK_RESTIC_DATA_DIR` overrides the configuration
directory, but does not isolate the OS keychain namespace.

Instances using `YSYNK_RESTIC_DATA_DIR` do not manage desktop-login autostart;
they must not change the installed application's startup registration. Test
autostart with isolated HOME/XDG paths and a private session bus, not the host's
login files. Minimized startup applies to `--autostart` launches only.

## Checks

```sh
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --locked --all-targets -- -D warnings
npm run check
npm test
npm run test:dependencies
npm run build
python3 -m unittest discover -s tools -p 'test_*.py'
python3 tools/check-backports.py
python3 tools/check_advisories.py --output docs/advisories.json
```

The advisory command returns a failure status when findings exist. The report
retains version findings for locally patched dependencies; see
[dependency notes](docs/DEPENDENCIES.md). Vendored GLib emits upstream compiler
warnings with the pinned toolchain.

Tests use temporary repositories and injected credentials. Never use an
existing user's keychain for automated tests. Native-provider tests require an
isolated session and disposable credential store. Screenshots in the README
use sample data, not evidence of a completed backup.

For offline browser regression checks, run `npm run build` followed by
`node ui/audit-fixture.mjs`. Open the generated `target/audit-ui/index.html` in
a separate browser tab and run `await verifyAuditUi()` in its console on a
fresh load. This exercises the real bundled UI with in-memory IPC responses;
it does not connect to the native engine or keychain. Check both audited
window sizes (1120×800 and 720×580), themes, keyboard navigation, console errors
and failed requests. Native integration still needs separate verification.

## Packaging

```sh
python3 tools/release-notices.py --fetch-missing
sh packaging/build-deb.sh
```

`--fetch-missing` downloads pinned upstream license texts when the local notice
cache is incomplete. Subsequent notice generation works offline. The Debian
package is written to `target/release/bundle/deb/`; building does not install it.
For an executable without a package, use
`npm run desktop:build -- --no-bundle`.

Linux x86-64 is the tested build target. Native picker interaction, live remote
backends, platform-specific credential behavior and independent recovery need
separate testing. macOS and Windows are not release targets yet.
