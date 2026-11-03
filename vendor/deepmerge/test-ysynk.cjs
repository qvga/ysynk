const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const test = require('node:test');
const vm = require('node:vm');

const read = (file) => fs.readFileSync(path.join(__dirname, file), 'utf8');
// The published source's sole dependency is bundled in its distributions.
// Reuse that exact helper to test index.js without installing devDependencies.
const bundled = read('dist/cjs.js');
const helper = vm.runInThisContext('(function () {' +
    bundled.slice(0, bundled.indexOf('function emptyTarget')) +
    '; return isMergeableObject; })()');
const sourceModule = { exports: {} };
vm.runInThisContext('(function (require, module, exports) {' + read('index.js') + '\n})')(
    (name) => {
        assert.equal(name, 'is-mergeable-object');
        return helper;
    }, sourceModule, sourceModule.exports);

const implementations = {
    source: sourceModule.exports,
    cjs: require('./dist/cjs.js'),
    umd: require('./dist/umd.js')
};

function assertPlain(result) {
    assert.equal(Object.getPrototypeOf(result), Object.prototype);
    assert.equal({}.polluted, undefined);
}

function assertOwnProto(result, expected) {
    assertPlain(result);
    assert.deepEqual(Object.getOwnPropertyDescriptor(result, '__proto__'), {
        value: expected, enumerable: true, configurable: true, writable: true
    });
    assert.equal(result.polluted, undefined);
}

for (const [name, merge] of Object.entries(implementations)) {
    test(`${name}: target own __proto__ does not change the result prototype`, () => {
        const target = JSON.parse('{"__proto__":{"polluted":true}}');
        const result = merge(target, {});
        assertOwnProto(result, { polluted: true });
        assert.equal(Object.getPrototypeOf(target), Object.prototype);
    });

    test(`${name}: source __proto__ merges into an existing own data property`, () => {
        const target = JSON.parse('{"__proto__":{"left":1}}');
        const source = JSON.parse('{"__proto__":{"right":2}}');
        assertOwnProto(merge(target, source), { left: 1, right: 2 });
        assertOwnProto(merge(target, JSON.parse('{"__proto__":7}')), 7);
    });

    test(`${name}: null-prototype input properties remain own data properties`, () => {
        const target = Object.create(null);
        const source = Object.assign(Object.create(null),
            JSON.parse('{"__proto__":{"polluted":true},"constructor":{"prototype":{"polluted":true}}}'));
        const result = merge(target, source);
        assertOwnProto(result, { polluted: true });
        assertPlain(result.constructor);
        assert.deepEqual(result.constructor.prototype, { polluted: true });
        assert.equal(Object.prototype.polluted, undefined);
        assertPlain(merge(null, { safe: true }));
        assert.throws(() => merge({}, null), TypeError);
    });

    test(`${name}: nested target data cannot poison nested result prototypes`, () => {
        const target = JSON.parse('{"nested":{"__proto__":{"polluted":true},"constructor":{"prototype":{"value":1}}}}');
        const result = merge(target, { nested: { safe: true } });
        assertPlain(result);
        assertOwnProto(result.nested, { polluted: true });
        assert.equal(result.nested.safe, true);
        assert.deepEqual(result.nested.constructor.prototype, { value: 1 });
        assert.equal({}.value, undefined);
        // Existing protection still skips unsafe inherited source keys.
        assert.deepEqual(merge({}, JSON.parse('{"__proto__":{"polluted":true},"constructor":{"prototype":{"polluted":true}}}')), {});
    });

    test(`${name}: ordinary nested merges clone and concatenate arrays`, () => {
        const target = { nested: { left: 1 }, values: [{ left: 1 }] };
        const source = { nested: { right: 2 }, values: [{ right: 2 }] };
        const result = merge(target, source);
        assert.deepEqual(result, {
            nested: { left: 1, right: 2 }, values: [{ left: 1 }, { right: 2 }]
        });
        assert.notEqual(result.nested, target.nested);
        assert.notEqual(result.values[0], target.values[0]);
        assertPlain(result);
    });

    test(`${name}: enumerable symbols merge and nonenumerable symbols stay absent`, () => {
        const symbol = Symbol('option');
        const hidden = Symbol('hidden');
        const target = { [symbol]: { left: 1 } };
        Object.defineProperty(target, hidden, { value: true });
        const result = merge(target, { [symbol]: { right: 2 } });
        assert.deepEqual(result[symbol], { left: 1, right: 2 });
        assert.equal(Object.hasOwn(result, hidden), false);
        assertPlain(result);
    });

    test(`${name}: Svelte options replace arrays and preserve function identity`, () => {
        const preprocess = () => {};
        const onwarn = () => {};
        const replacements = [preprocess];
        const result = merge(
            { preprocess: [() => {}], compilerOptions: { dev: false } },
            { preprocess: replacements, onwarn, compilerOptions: { dev: true } },
            { arrayMerge: (target, source) => source ?? target }
        );
        assert.equal(result.preprocess, replacements);
        assert.equal(result.preprocess[0], preprocess);
        assert.equal(result.onwarn, onwarn);
        assert.deepEqual(result.compilerOptions, { dev: true });
        assertPlain(result);
    });
}

test('source and both distributions have identical mergeObject logic', () => {
    const body = (text) => text.slice(text.indexOf('function mergeObject('),
        text.indexOf('function deepmerge(')).replace(/[\s;]/g, '');
    assert.equal(body(read('index.js')), body(read('dist/cjs.js')));
    assert.equal(body(read('index.js')), body(read('dist/umd.js')));
});
