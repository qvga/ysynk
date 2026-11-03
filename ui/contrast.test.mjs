import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

const css = readFileSync(new URL('./src/style.css', import.meta.url), 'utf8');
function luminance(hex) {
  const channels = hex.match(/../g).map(value => parseInt(value, 16) / 255).map(value => value <= .04045 ? value / 12.92 : ((value + .055) / 1.055) ** 2.4);
  return channels[0] * .2126 + channels[1] * .7152 + channels[2] * .0722;
}
function contrast(a, b) {
  const values = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (values[0] + .05) / (values[1] + .05);
}

describe('form control boundaries', () => {
  it('distinguishes enabled controls from both adjacent backgrounds in each theme', () => {
    const themes = [...css.matchAll(/\.app(?:\[data-theme="(?:dark|system)"\])?\s*\{([^}]+)\}/g)].filter(([, declarations]) => declarations.includes('--bg:'));
    expect(themes).toHaveLength(3);
    for (const [, declarations] of themes) {
      const token = name => declarations.match(new RegExp(`--${name}: #([0-9a-f]{6})`))?.[1];
      const boundary = token('control-border');
      expect(boundary).toBeDefined();
      for (const background of ['surface', 'bg']) expect(contrast(boundary, token(background))).toBeGreaterThanOrEqual(3);
    }
    expect(css).toMatch(/input, textarea, select\s*\{[^}]*border: 1px solid var\(--control-border\)/);
  });
});
