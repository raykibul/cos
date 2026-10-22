// The width of the chief's terminal split, as the divider sets it: kept in
// pixels in this browser's storage, and clamped so neither side gets narrower
// than MIN_SIDE. Storage may be missing or refuse (a private window), so each
// read and write is wrapped and the split still works without it.

export const SPLIT_KEY = "cos.chief.terminal-width";
/** Narrower than this and the conversation's composer or Claude's screen wraps badly. */
export const MIN_SIDE = 360;
/** Without a saved width: 46% of the page, at most this. */
const DEFAULT_MAX = 760;
const DEFAULT_SHARE = 0.46;

/** `width` kept between MIN_SIDE and what leaves the conversation MIN_SIDE; unmeasured (0) only has a floor. */
export function clampWidth(width: number, total: number): number {
  const max = total > 0 ? Math.max(MIN_SIDE, total - MIN_SIDE) : Number.POSITIVE_INFINITY;
  return Math.round(Math.min(Math.max(width, MIN_SIDE), max));
}

/** The split before anyone drags it, as it always was: 46% of the page, 360 to 760 px. */
export function defaultWidth(total: number): number {
  return clampWidth(Math.min(total * DEFAULT_SHARE, DEFAULT_MAX), total);
}

export function savedWidth(): number | null {
  try {
    const value = Number(window.localStorage.getItem(SPLIT_KEY));
    return Number.isFinite(value) && value > 0 ? value : null;
  } catch {
    return null;
  }
}

/** Saves the width, or forgets it with null (back to the default). */
export function saveWidth(width: number | null): void {
  try {
    if (width === null) window.localStorage.removeItem(SPLIT_KEY);
    else window.localStorage.setItem(SPLIT_KEY, String(Math.round(width)));
  } catch {
    // Not kept for next time; the split itself still works.
  }
}
