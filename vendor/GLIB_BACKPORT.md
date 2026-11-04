# GLib 0.18.5 downstream backport

`glib/` contains the published **glib 0.18.5** crate, with one upstream
correctness fix backported by this project and the compiler-warning cleanup
described below. This is not a new upstream release,
an upstream-supported 0.18 security release, or a claim that a version-based
advisory scanner will recognize the fix. The package version remains `0.18.5`.
The root Cargo patch selects this local source for the existing GTK dependency
chain; migration to a supported upstream stack remains separate work.

## Provenance and exact scope

- Published archive: <https://static.crates.io/crates/glib/glib-0.18.5.crate>.
  Its SHA-256 is
  `233daaf6e83ae6a12a52055f568f9d7cf4671dabb78ff9560ab6da230ce00ee5`,
  matching the registry checksum in `Cargo.lock` before the local path patch.
  Both the downloaded public archive and the existing Cargo cache archive were
  checked against that checksum.
- Published `.cargo_vcs_info.json` records source commit
  `42b9caf98e03ded086362d9653ca58fe94dc8658`, path `glib`.
- Official fix: gtk-rs/gtk-rs-core commit
  [`b5a4071e439bef2b5eea76c3aa25e5ae84839e34`](https://github.com/gtk-rs/gtk-rs-core/commit/b5a4071e439bef2b5eea76c3aa25e5ae84839e34),
  “glib: fix UB in VariantStrIter::impl_get”, dated 2024-03-30.
  The [primary patch](https://github.com/gtk-rs/gtk-rs-core/commit/b5a4071e439bef2b5eea76c3aa25e5ae84839e34.patch)
  downloaded for comparison has SHA-256
  `a742bf59f1b00af0f434623b34e2fd969dc8697d37c796efdf909dd9eb19b854`.
- All 121 published files are retained, including the original normalized
  `Cargo.toml`, `Cargo.toml.orig`, README, COPYRIGHT, MIT LICENSE, source, tests,
  and benchmark/configuration files. No licensing terms were changed.
- The correctness backport changes only `glib/src/variant_iter.rs`.
  `VariantStrIter::impl_get` changes `let p` to `let mut p` and `&p` to
  `&mut p`. This permits the FFI function to write its output pointer without
  writing through an immutable Rust reference. The hunk span and complete body
  were compared with the official patch and match exactly (ignoring only Git's
  function-context suffix on the hunk header).
- Original `src/variant_iter.rs` SHA-256:
  `1fd02859333761c45321b32f28b24233446b97d0022a90d3a937ed162585b90e`.
  Patched SHA-256:
  `a0f5ee8acb8faa089bcdfbc9a57372609fce7654026ccef7d9a224d05a654ccc`.
- The only added crate file is `tests/variant_str_iter_backport.rs`.
  It covers `next`, `nth`, `next_back`, `nth_back`, `last`, and `collect`,
  including empty and non-ASCII string values. This document is downstream-only.

## Compiler-warning cleanup

The pinned Rust 1.91 toolchain reports 34 warnings in the published source.
Downstream syntax-only changes apply its suggestions without suppressing lints:

- `src/value_array.rs`: remove three redundant parentheses around `dyn FnMut`
  types.
- `src/collections/list.rs`, `src/collections/slist.rs`, `src/enums.rs`,
  `src/main_context.rs`, `src/object.rs`, `src/param_spec.rs`,
  `src/subclass/types.rs`, `src/translate.rs`, `src/value.rs`, `src/variant.rs`,
  and `src/variant_type.rs`: make 31 existing inferred lifetimes explicit with
  `'_` or the already-named `'a` lifetime.

These edits do not change behavior, dependency versions, or the correctness
backport. All other published files remain unchanged. The expected SHA-256
hashes for these 12 files and the correctness backport are retained in
`tools/check-backports.py`.

## Verification

Run the focused regression and upstream iterator tests from the repository root:

```sh
cargo test --release --manifest-path vendor/glib/Cargo.toml --test variant_str_iter_backport
cargo test --release --manifest-path vendor/glib/Cargo.toml --lib variant_iter::tests
python3 tools/check-backports.py
```

The optimized regression targets the pointer mutation that caused undefined
behavior in the original implementation. Review this backport and remove the
local override when the desktop dependency chain supports an upstream fix.
