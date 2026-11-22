# Develop ysynk

## Prepare a Linux checkout

Use the Rust toolchain in `rust-toolchain.toml`, the Node.js version in `.nvmrc`, and Python 3.11 or newer. Native builds need a C compiler, pkg-config, GTK3, WebKitGTK 4.1, AppIndicator, and D-Bus development headers. The Linux checks use `/usr/bin/restic` (0.17.1 or newer) and `/usr/bin/python3`. The [Linux checks workflow](.github/workflows/check.yml) lists the Ubuntu packages.

```sh
npm ci
cargo fetch --locked --target x86_64-unknown-linux-gnu
python3 tools/release-notices.py --fetch-missing
./dev.sh
```

The first three commands fetch the locked dependencies and prepare the license texts required by the native bundle. Subsequent notice generation uses the local cache. `dev.sh` builds the bundled frontend and launches a debug executable. For hot reload, run `npm run dev` and `npm run desktop` in separate terminals after preparing the checkout.

To register a local application-menu entry, run `python3 tools/install-desktop-entry.py` and reopen ysynk. Pass the executable path explicitly when using a custom Cargo target directory. The Debian package installs its own launcher.

## Where changes belong

| Directory | Responsibility |
| --- | --- |
| `crates/core/src/` | Configuration, keychain records, job admission and persistence, restic subprocesses, recovery, and retention |
| `desktop/src/` | Native dialogs and IPC, application lifecycle, notifications, tray, and desktop-login settings |
| `ui/src/` | Svelte interface, result validation, job polling, and operation history |
| `tools/`, `packaging/` | Checks, license notices, desktop integration, and Linux builds |
| `vendor/` | Retained upstream sources with documented downstream patches |

A repository ID and its exact location bind a keychain credential. Preserve both when reading existing configurations. The core owns admission of repository operations and returns session results to the renderer. Changes to job state, output handling, or destructive operations should be tested through the request interface. [Security and recovery](docs/SECURITY.md)

For workflow changes, the [state-machine map](docs/state-machines.md) traces job admission, cancellation and recovery across layers and lists the owner, guards and checks for each workflow.

## Run the checks

```sh
sh tools/check.sh
node ui/audit-fixture.mjs
xvfb-run -a /usr/bin/python3 tools/test-webview.py
```

The first command runs formatting, strict Clippy, workspace tests, Svelte and TypeScript checks, frontend and Python tests, the production frontend build, and local backport checks. The WebKitGTK test exercises the bundled UI at 1120×800 and 720×580 using in-memory IPC responses. It requires Xvfb, Xauth, PyGObject and the GTK3 and WebKitGTK 4.1 introspection packages. You can also open `target/audit-ui/index.html` in a browser and run `await verifyAuditUi()` on a fresh load.

Core integration tests use disposable repositories and injected credentials. For manual native-provider or desktop-login tests, use a private session bus, temporary XDG directories and a disposable keychain. `YSYNK_RESTIC_DATA_DIR` isolates configuration and disables login registration, but the credential namespace remains shared. A Linux AppIndicator deprecation message can appear at startup even when the tray works.

After changing a lockfile, refresh the external advisory inventory:

```sh
python3 tools/check_advisories.py --output docs/advisories.json
```

Exit status 0 means the query returned no matches, 1 means it found matches, and 2 means the scan could not complete. Review the raw findings alongside the local patch verification. [Dependency notes](docs/DEPENDENCIES.md)

## Build release assets

```sh
python3 packaging/build-release.py
```

The command builds the Debian package and assembles a standalone Linux x86-64 archive with the launcher helper, icon, and licenses. It prints the fresh directory containing both downloads and `SHA256SUMS`. Each build gets its own directory. The asset build runs locally and stops after checking the hashes.

For just the Debian package, run `sh packaging/build-deb.sh`. For just the executable, run `npm run desktop:build -- --no-bundle`. The archive uses the standalone executable because Tauri sets a Debian-specific marker in the packaged copy.

When preparing a release, update the workspace, npm and Tauri versions and their lockfiles together. Update the installation example and write release notes in `docs/releases/`. After publishing, check that the tag points to the tested commit and verify the downloaded assets against the locally generated checksums.

[Historical November 2026 audit](docs/history/2026-11-03-audit.md) · [Dependency patch provenance](docs/DEPENDENCIES.md#retained-source-fixes)
