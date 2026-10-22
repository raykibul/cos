// What the mouse wheel sends to a full-screen app (the alternate screen).
//
// ghostty-web 0.4.0 always turns the wheel into arrow keys there. Claude
// Code turns on mouse reporting (modes 1003 and 1006) and expects wheel
// reports, so arrows made it step through prompt history instead of
// scrolling. On the normal screen the wheel scrolls the scrollback, which
// ghostty-web already does.

export interface WheelState {
  /** Wheel movement in lines, signed, not sent yet. */
  carry: number;
}

export interface WheelInput {
  deltaY: number;
  /** `WheelEvent.deltaMode`: 0 pixels, 1 lines, 2 pages. */
  deltaMode: number;
  /** Pointer position inside the terminal, in pixels. */
  x: number;
  y: number;
  width: number;
  height: number;
  cols: number;
  rows: number;
  /** The app asked for mouse reports (any of modes 1000, 1002, 1003). */
  mouseTracking: boolean;
  /** SGR encoding (mode 1006). */
  sgr: boolean;
}

/** At most this many reports for one wheel event, so a fling stays sane. */
const MAX_STEPS = 10;
/** Without mouse reporting, one Page Up or Down per this share of the screen. */
const PAGE_SHARE = 1 / 3;

const PAGE_UP = "\x1b[5~";
const PAGE_DOWN = "\x1b[6~";

/**
 * The bytes to send for one wheel event on the alternate screen, and the
 * movement left over for the next one.
 */
export function wheelData(input: WheelInput, state: WheelState): string {
  const cellHeight = input.rows > 0 ? input.height / input.rows : 16;
  const lines =
    input.deltaMode === 1
      ? input.deltaY
      : input.deltaMode === 2
        ? input.deltaY * input.rows
        : input.deltaY / Math.max(cellHeight, 1);
  state.carry += lines;

  if (input.mouseTracking) {
    const steps = Math.min(Math.trunc(Math.abs(state.carry)), MAX_STEPS);
    if (steps === 0) return "";
    const down = state.carry > 0;
    state.carry -= Math.sign(state.carry) * Math.trunc(Math.abs(state.carry));
    const report = wheelReport(input, down);
    return report.repeat(steps);
  }

  const perPage = Math.max(3, Math.round(input.rows * PAGE_SHARE));
  const pages = Math.min(Math.trunc(Math.abs(state.carry) / perPage), MAX_STEPS);
  if (pages === 0) return "";
  const down = state.carry > 0;
  state.carry -= Math.sign(state.carry) * pages * perPage;
  return (down ? PAGE_DOWN : PAGE_UP).repeat(pages);
}

/** One wheel report at the pointer's cell (1-based). */
function wheelReport(input: WheelInput, down: boolean): string {
  const cellWidth = input.cols > 0 ? input.width / input.cols : 8;
  const cellHeight = input.rows > 0 ? input.height / input.rows : 16;
  const col = clamp(Math.floor(input.x / Math.max(cellWidth, 1)) + 1, 1, Math.max(input.cols, 1));
  const row = clamp(Math.floor(input.y / Math.max(cellHeight, 1)) + 1, 1, Math.max(input.rows, 1));
  const button = down ? 65 : 64;
  if (input.sgr) return `\x1b[<${button};${col};${row}M`;
  // The old X10 encoding: each value plus 32, as one byte, so it caps at 223.
  const byte = (value: number) => String.fromCharCode(Math.min(value + 32, 255));
  return `\x1b[M${byte(button)}${byte(col)}${byte(row)}`;
}

function clamp(value: number, low: number, high: number): number {
  return Math.min(Math.max(value, low), high);
}
