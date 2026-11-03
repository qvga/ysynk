# deepmerge 4.3.1: downstream security patch

`deepmerge/` is the published npm **deepmerge 4.3.1** package with a local fix
for [GHSA-gcqx-w48m-w2h4](https://github.com/advisories/GHSA-gcqx-w48m-w2h4).
This is a **downstream security patch**, not an official fixed release or an
upstream-supported backport. Its package version remains `4.3.1`; version-based
advisory tools may continue to report it. The workspace selects this
directory through its npm override without replacing the official Svelte plugin.

## Provenance

- Registry metadata: <https://registry.npmjs.org/deepmerge/4.3.1>.
- Published archive: <https://registry.npmjs.org/deepmerge/-/deepmerge-4.3.1.tgz>.
- Original archive SHA-256:
  `4003a0a348276a95543fa969f561f6b5968629acbbb59d66018dd23e7b8f1d5c`.
- Registry SHA-1: `44b5f2147cd3b00d4b56137685966f26fd25dd4a`.
- Registry integrity:
  `sha512-3sUqbMEc77XqpdNO7FRyRog+eW3ph+GYCbj+rK+uYyRMuwsVy0rMiVtPn+QJlKFvWP/1PYpapqYn0Me2knFn+A==`.
  Both registry digests were checked against the downloaded archive.
- All published source, distributions, typings, package metadata, documentation,
  Rollup configuration and MIT `license.txt` are retained. Only the published
  `.eslintcache` is omitted; no node_modules or development dependencies are
  vendored. Licensing and the true package version are unchanged. Integration
  additionally removes upstream development scripts/devDependencies from the
  local `package.json`, because npm file links otherwise install the obsolete
  upstream development tree. Runtime entry points and engines remain unchanged.

## Exact change and maintenance responsibility

The published source is `index.js`, not `src/index.js`. In `mergeObject`, each
of the three `destination[key] = value` writes (target clone, recursive source
merge, source clone) becomes:

```js
Object.defineProperty(destination, key, {
    value: /* the unchanged original value expression */,
    enumerable: true, configurable: true, writable: true
})
```

The actual code contains the original expression, not this explanatory comment.
The same manual change is applied to `dist/cjs.js` and `dist/umd.js`. A parity
test compares the complete `mergeObject` implementations, normalizing only
whitespace and distribution-generated semicolons. No other production logic is
changed; distribution rebuilds must preserve this patch.

An own enumerable `__proto__` property on a target previously invoked the
prototype setter on the fresh `{}` destination. Source writes could also invoke
it when the target made that key acceptable to the existing source-key guard.
Defining own data properties avoids the setter in all three paths, preserves
the ordinary `Object.prototype` result prototype, and retains the existing
source-key filtering, merge callbacks, array behavior and property attributes.
Permitted `__proto__` data remains an own property; it is not a prototype.

SHA-256 of changed files (published → patched):

| File | Published | Patched |
| --- | --- | --- |
| `index.js` | `12440c84957c1990ea2d5e33dc0091722dc1dc1585c6d20e0c780e568ffa9d30` | `92d5af8bf2c1a5f75929941600ab839f1473296d4d2bd7d7a2a57389e6528324` |
| `dist/cjs.js` | `2e5160f84a1141bd927264afe4d15766f1e6cf07daef182b37ff53fea121de7d` | `88121122451e38da55a2c841a30b5abb9d6402a32216d92cbca26eafd520f839` |
| `dist/umd.js` | `f33e6596482d1fb5c7c427f8ef92f2aa849eea9223acd974176b2fb27ddbf73a` | `24deeddeecdfa685d2e89da821e8e98ebfe3ceadf0d4da01bb2ed4fe2f22d5af` |

The original registry integrity is evidence for the original archive only,
not an integrity value for the modified tree.

This project owns review and maintenance of this local fix. Remove the override when a reviewed
upstream solution is available; do not relabel this as a fixed upstream version.

## Verification

Run from the repository root using the `.nvmrc` runtime:

```sh
node --test vendor/deepmerge/test-ysynk.cjs
node --check vendor/deepmerge/index.js
node --check vendor/deepmerge/dist/cjs.js
node --check vendor/deepmerge/dist/umd.js
python3 tools/check-backports.py
npm run test:dependencies
```

Coverage includes target/source/nested `__proto__`, null-prototype inputs,
constructor/prototype data, unchanged source-key filtering, unchanged null-input
behavior, normal nested clones, arrays, enumerable symbols, replacement arrays
and function-valued Svelte options. Assertions check result prototypes, own
data descriptors and absence of ambient `{}.polluted`.

The source test reuses the exact `is-mergeable-object` helper bundled in CJS,
without installing the package's development-only dependency.
`npm run test:dependencies` checks both the regressions and actual resolution from
the Svelte plugin. Keep the root file dependency and `$deepmerge` override together;
an override-only relative path can resolve inside the plugin rather than to the
reviewed vendored source.
