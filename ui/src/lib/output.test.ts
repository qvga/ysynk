import { describe, expect, it } from 'vitest';
import { atOutputBottom, outputText, outputRetentionNotice } from './output';

describe('command output', () => {
  it('preserves literal markup, whitespace, and long lines without parsing them', () => {
    const lines = ['  opening repository', '<img src=x onerror=alert(1)>', 'x'.repeat(1024)];
    expect(outputText(lines)).toBe(lines.join('\n'));
    expect(outputText()).toBe('');
  });

  it('changes text when a full ring replaces its oldest line', () => {
    const lines = Array.from({ length: 200 }, (_, i) => `file ${i}`);
    expect(outputText([...lines.slice(1), 'file 200'])).not.toBe(outputText(lines));
  });

  it('follows only at the bottom with a small rounding allowance', () => {
    expect(atOutputBottom({ scrollHeight: 500, clientHeight: 100, scrollTop: 400 })).toBe(true);
    expect(atOutputBottom({ scrollHeight: 500, clientHeight: 100, scrollTop: 398.5 })).toBe(true);
    expect(atOutputBottom({ scrollHeight: 500, clientHeight: 100, scrollTop: 380 })).toBe(false);
    expect(atOutputBottom({ scrollHeight: 100, clientHeight: 100, scrollTop: 0 })).toBe(true);
  });

  it('explains the output retention limits', () => {
    expect(outputRetentionNotice).toContain('200 lines / 64 KiB');
    expect(outputRetentionNotice).toContain('1,024 bytes');
    expect(outputRetentionNotice).toContain('session only');
  });
});
