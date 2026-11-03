// Verify the actual plugin resolution, not merely a separately tested vendor copy.
const { createRequire } = require('node:module');
const { realpathSync } = require('node:fs');
const { resolve } = require('node:path');
const assert = require('node:assert/strict');
const fromPlugin = createRequire(require.resolve('@sveltejs/vite-plugin-svelte'));
assert.equal(realpathSync(fromPlugin.resolve('deepmerge')), realpathSync(resolve(__dirname, '../vendor/deepmerge/dist/cjs.js')));
const merge = fromPlugin('deepmerge');
const result = merge(JSON.parse('{"__proto__":{"polluted":true}}'), {});
assert.equal(Object.getPrototypeOf(result), Object.prototype);
assert.equal(result.polluted, undefined);
assert.equal({}.polluted, undefined);
console.log('Svelte plugin resolves the patched deepmerge; prototype regression passed.');
