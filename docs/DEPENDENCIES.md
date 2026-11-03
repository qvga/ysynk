# Dependencies

Exact dependency versions and integrity records are in the manifests and
lockfiles. Rust, Svelte 5 and Tauri 2 form the application; external restic owns
the backup format, encryption, transport and repository locking.

Native credential libraries provide persistent Secret Service on Linux,
Keychain on macOS and Credential Manager on Windows. Backend features must be
explicit: a library's mock/default or kernel-only fallback is not acceptable
for production secrets.

Linux deliberately disables the Secret Service library's optional crypto
features and uses Plain sessions on a trusted local user bus. Version 4.1.0's
DH private keys and CBC IVs use non-cryptographic randomness; enabling either
crypto backend is not an acceptable fix. See the transport boundary in
[security](SECURITY.md).

## Retained source fixes

- `vendor/glib`: true version 0.18.5, with the official mutable-output-pointer
  backport for VariantStrIter. See [provenance](../vendor/GLIB_BACKPORT.md).
- `vendor/deepmerge`: true version 4.3.1, with the project-maintained own-data-
  property fix in source and distributions. See
  [provenance](../vendor/DEEPMERGE_BACKPORT.md).

These are not upstream-supported patched releases. The raw advisory report
retains version findings; passing source checks is not a clean-advisory claim
or independent audit. `python3 tools/check-backports.py` checks retained bytes
and selection; `npm run test:dependencies` checks real plugin resolution and
merge regressions. Re-run `tools/check_advisories.py` after lockfile changes.
Network failure means unavailable, not zero findings.

The advisory inventory uses upstream npm names for aliases and queries local
backports from their manifests rather than their filesystem paths. Unsupported
or incomplete npm lockfile inventories fail the check. `npm audit` does not
replace this query: it can report zero while local-file version findings remain.

The single-instance plugin currently uses zbus 5. Its standalone `zbus_names`,
`zvariant` and `zvariant_derive` dependencies have unmaintained-package
advisories (RUSTSEC-2026-0371, RUSTSEC-2026-0370 and RUSTSEC-2026-0372) because
they were merged into zbus 6. They remain in the pinned plugin's dependency
graph; the raw report retains these findings, alongside `proc-macro-error`'s
unmaintained-package advisory. Updating that plugin to a zbus-6-based release
is separate dependency maintenance, not a suppressed finding.

External restic and its compiled Go runtime are not covered by Cargo/npm
lockfile checks. `docs/audit-runtime-advisories.json` records the audit host's
version-only results, not verified reachable vulnerabilities or an inventory
of every installation. Keep the system restic package and its compiler/runtime
security updates current; evaluate distribution backports before interpreting
version matches. This audit did not change system packages.

## Notices

ysynk remains Apache-2.0. External restic retains its BSD 2-Clause notice in
`docs/RESTIC-LICENSE`. `python3 tools/release-notices.py` creates an intentionally
overinclusive locked Linux Cargo/JavaScript notice inventory. Missing notices
block packaging; `--fetch-missing` explicitly prepares an upstream cache when
needed. Packages include the application license, restic notice and third-party
inventory. Distribution still requires platform and license review.
