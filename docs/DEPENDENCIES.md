# Dependencies

ysynk uses Rust for the backup engine, Svelte 5 for the interface and Tauri 2 for the desktop application. Cargo and npm lockfiles pin the dependencies used to build it. The installed restic executable, GTK, WebKitGTK and the desktop keychain are system dependencies. Update them through the operating system.

Linux credential storage uses the persistent Secret Service collection over a trusted local user bus. The pinned `dbus-secret-service` 4.1.0 library uses non-cryptographic randomness for its optional Diffie–Hellman private key and IV generation. ysynk disables those features and uses Plain sessions. Native credential backends are selected explicitly. Mock, kernel-only and plaintext stores are excluded from production credential storage. [Credential boundary](SECURITY.md#credential-boundary)

## Retained source fixes

| Dependency | Local change | Verification |
| --- | --- | --- |
| GLib 0.18.5 | The official fix for the output pointer in `VariantStrIter`, plus syntax changes for compiler warnings. Cargo selects this copy for the GTK dependency chain. | [Patch, checksums and iterator tests](../vendor/GLIB_BACKPORT.md) |
| deepmerge 4.3.1 | Object merges define own data properties, including for keys named `__proto__`. npm selects the patched code for the Svelte plugin. | [Patch, checksums and merge tests](../vendor/DEEPMERGE_BACKPORT.md) |

Both copies retain their published upstream versions. ysynk maintains these backports until the dependency graph can use suitable upstream releases. From the repository root, check the reviewed source bytes and the plugin's installed dependency with these commands.

```sh
python3 tools/check-backports.py
npm run test:dependencies
```

### Advisory checks

After a lockfile change, run the public OSV query and review the resulting diff.

```sh
python3 tools/check_advisories.py --output docs/advisories.json
```

The scanner queries public package names and pinned versions. It resolves npm aliases to upstream names and verifies linked packages against their local manifests. Incomplete npm records, partial API responses and network failures stop the scan. Exit status `0` means the query completed with no matches, `1` means matches were recorded, and `2` means the check was unavailable. Version matches stay in the report even when a local patch addresses them. Source verification and regression tests establish what shipped.

The November 22, 2026 scan returned matches for six of 575 packages. [Full lockfile report](advisories.json)

- **GLib.** The two identifiers refer to the same iterator issue, covered by the verified backport. The scan also included deepmerge and returned no match for its version. [GLib advisory](https://osv.dev/vulnerability/GHSA-wrw7-89jp-8q8g)
- **Maintenance advisories.** `proc-macro-error` and `chrono` carry unmaintained-package advisories. `chrono` 0.4.45 appears in the lockfile but was absent from the active Linux x86-64 build graph at review time. The other three findings concern `zbus_names`, `zvariant` and `zvariant_derive` in the single-instance plugin's zbus 5 dependency tree. They were merged into zbus 6. Moving to the newer stack requires a compatible plugin. [chrono advisory](https://rustsec.org/advisories/RUSTSEC-2026-0543.html)
- **System restic.** A separate November 3, 2026 snapshot records version matches for restic and the Go runtime on the audit host. Evaluate the installed binary, distribution backports and reachable code before drawing conclusions about another machine. [Runtime report](audit-runtime-advisories.json)

## Notices

ysynk ships under Apache-2.0. The Linux packages include the ysynk license, restic's BSD 2-Clause notice and a third-party notice inventory. The inventory includes build and development dependencies from the locked Linux Cargo graph and the specified JavaScript packages. It deliberately covers more code than the final executable links. [ysynk license](../LICENSE) · [restic license](RESTIC-LICENSE)

Packaging runs `python3 tools/release-notices.py` and stops if required notices are missing. Use `--fetch-missing` to prepare the pinned upstream notice cache. Review distribution and platform obligations when changing dependencies. [Packaging instructions](../CONTRIBUTING.md#build-release-assets)
