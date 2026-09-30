/** What a key press means to a hotkey box. */
export type HotkeyKey =
  | { kind: 'ignore' }
  | { kind: 'cancel' }
  | { kind: 'clear' }
  | { kind: 'refused' }
  | { kind: 'combo'; hotkey: string };

const MODIFIER_CODES = new Set(['ControlLeft', 'ControlRight', 'AltLeft', 'AltRight', 'ShiftLeft', 'ShiftRight', 'MetaLeft', 'MetaRight', 'AltGraph']);

/** The key part of the canonical form (`A`-`Z`, `0`-`9`, `F1`-`F24`) from `KeyboardEvent.code`, or null. */
export function keyName(code: string): string | null {
  const letter = /^Key([A-Z])$/.exec(code);
  if (letter) return letter[1];
  const digit = /^Digit([0-9])$/.exec(code);
  if (digit) return digit[1];
  const fn = /^F([1-9]\d?)$/.exec(code);
  return fn && Number(fn[1]) <= 24 ? `F${fn[1]}` : null;
}

/**
 * Reads one key press: Esc cancels, Delete or Backspace clears, a lone modifier or Tab is left
 * alone, and anything else is either a canonical `Ctrl+Alt+Shift+<Key>` with at least two
 * modifiers or refused.
 */
export function readHotkeyKey(event: Pick<KeyboardEvent, 'code' | 'ctrlKey' | 'altKey' | 'shiftKey' | 'metaKey'>): HotkeyKey {
  if (MODIFIER_CODES.has(event.code) || event.code === 'Tab') return { kind: 'ignore' };
  if (event.code === 'Escape') return { kind: 'cancel' };
  if (event.code === 'Delete' || event.code === 'Backspace') return { kind: 'clear' };
  const key = keyName(event.code);
  const modifiers = [event.ctrlKey && 'Ctrl', event.altKey && 'Alt', event.shiftKey && 'Shift'].filter(Boolean) as string[];
  if (key === null || event.metaKey || modifiers.length < 2) return { kind: 'refused' };
  return { kind: 'combo', hotkey: [...modifiers, key].join('+') };
}
