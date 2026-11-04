/** Preserve native, sanitized lines as text, including whitespace and literal markup. */
export const outputText = (lines: readonly string[] = []) => lines.join('\n');

/** Allow only subpixel rounding at the bottom, not a whole unread line. */
export const atOutputBottom = (view: { scrollHeight: number; clientHeight: number; scrollTop: number }) =>
  view.scrollHeight - view.clientHeight - view.scrollTop <= 2;

export const outputRetentionNotice = 'Earlier output omitted. Only the latest 200 lines / 64 KiB are retained; each line is limited to 1,024 bytes. Output is kept for this session only.';
