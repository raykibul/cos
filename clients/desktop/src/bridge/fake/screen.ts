// A Claude Code screen in the style of 2.x, as raw terminal bytes: the
// welcome box, a prompt, tool calls with a diff, then a permission dialog,
// the trust dialog, a spinner or the input box. Hand-made to look like a
// captured one (24-bit colours, box drawing, CRLF line ends), since running
// the real CLI costs tokens.

const ESC = "\u001b[";
const rgb = (r: number, g: number, b: number) => `${ESC}38;2;${r};${g};${b}m`;
const bg = (r: number, g: number, b: number) => `${ESC}48;2;${r};${g};${b}m`;
const reset = `${ESC}0m`;
const bold = `${ESC}1m`;
const dim = `${ESC}2m`;
const italic = `${ESC}3m`;
const orange = rgb(215, 119, 87);
const gray = rgb(153, 153, 153);
const green = rgb(78, 186, 101);
const blue = rgb(177, 185, 249);
const white = rgb(255, 255, 255);

/** The width a line takes on screen, without escape sequences. */
export function visibleWidth(line: string): number {
  // eslint-disable-next-line no-control-regex
  return [...line.replace(/\u001b\[[0-9;?]*[A-Za-z]/g, "")].length;
}

function box(lines: string[], width: number, color: string): string[] {
  const bar = "─".repeat(width - 2);
  const out = [`${color}╭${bar}╮${reset}`];
  for (const line of lines) {
    const pad = Math.max(0, width - 4 - visibleWidth(line));
    out.push(`${color}│${reset} ${line}${" ".repeat(pad)} ${color}│${reset}`);
  }
  out.push(`${color}╰${bar}╯${reset}`);
  return out;
}

export type Footer =
  | { kind: "dialog"; command: string; description: string }
  | { kind: "trust" }
  | { kind: "working"; verb: string; seconds: number }
  | { kind: "idle" }
  | { kind: "exited" };

export interface ScreenSpec {
  cwd: string;
  prompt: string;
  transcript: "login" | "limiter" | "review" | "plan" | "notes" | "blank";
  footer: Footer;
  cols: number;
  rows: number;
}

export interface Screen {
  /** Bytes that redraw the whole screen, as a snapshot would. */
  data: string;
  /** 1-based row and column of the input box's text, if it shows. */
  input: { row: number; col: number; width: number } | null;
  /** 1-based row of the spinner line, if it shows. */
  spinnerRow: number | null;
}

function transcript(kind: ScreenSpec["transcript"], prompt: string): string[] {
  const lines = [`${gray}>${reset} ${prompt}`, ""];
  switch (kind) {
    case "login":
      return [
        ...lines,
        `${white}⏺${reset} I'll start by looking at how sessions are created.`,
        "",
        `${green}⏺${reset} ${bold}Read${reset}(src/auth/session.ts)`,
        `  ⎿  Read ${bold}84${reset} lines ${gray}(ctrl+r to expand)${reset}`,
        "",
        `${green}⏺${reset} ${bold}Update${reset}(src/auth/session.ts)`,
        `  ⎿  Updated ${bold}src/auth/session.ts${reset} with ${bold}1${reset} addition and ${bold}1${reset} removal`,
        `       ${dim}11${reset}    export const SESSION_COOKIE = "sid";`,
        `       ${dim}12${reset} ${bg(92, 34, 41)}-  const TTL_MS = 5 * 60 * 1000;      ${reset}`,
        `       ${dim}12${reset} ${bg(34, 92, 50)}+  const TTL_MS = 5 * 60 * 60 * 1000; ${reset}`,
        `       ${dim}13${reset}    `,
        "",
      ];
    case "limiter":
      return [
        ...lines,
        `${white}⏺${reset} The bucket lives in src/limit.rs; I'll wire it into the login handler.`,
        "",
        `${green}⏺${reset} ${bold}Read${reset}(src/routes/login.rs)`,
        `  ⎿  Read ${bold}142${reset} lines ${gray}(ctrl+r to expand)${reset}`,
        "",
        `${green}⏺${reset} ${bold}Update${reset}(src/routes/login.rs)`,
        `  ⎿  Updated ${bold}src/routes/login.rs${reset} with ${bold}9${reset} additions`,
        `       ${dim}31${reset} ${bg(34, 92, 50)}+  if !limiter.try_take(&ip) {                     ${reset}`,
        `       ${dim}32${reset} ${bg(34, 92, 50)}+      return Err(TooManyRequests::after(60));     ${reset}`,
        `       ${dim}33${reset} ${bg(34, 92, 50)}+  }                                               ${reset}`,
        "",
        `${green}⏺${reset} ${bold}Bash${reset}(cargo test login)`,
        `  ⎿  test login::allows_five_attempts ... ${green}ok${reset}`,
        `     test login::sixth_attempt_is_429 ... ${green}ok${reset}`,
        `     ${gray}… +12 lines (ctrl+r to expand)${reset}`,
        "",
      ];
    case "review":
      return [
        ...lines,
        `${green}⏺${reset} ${bold}Bash${reset}(git diff --stat main...HEAD)`,
        `  ⎿   src/routes/login.rs | 9 ${green}+++++++++${reset}`,
        `      1 file changed, 9 insertions(+)`,
        "",
        `${white}⏺${reset} The limiter is taken before the password check, so a wrong password`,
        `  still costs a token. Retry-After is set. Tests cover the sixth attempt.`,
        "",
      ];
    case "plan":
      return [
        ...lines,
        `${green}⏺${reset} ${bold}cos - board_get${reset} ${gray}(MCP)${reset}`,
        `  ⎿  ${gray}Run 10 · running · 1 of 5 merged${reset}`,
        "",
        `${white}⏺${reset} Task 3 is blocked: the layer takes a token before auth. I'll ask`,
        `  whether to key buckets per user or per IP before retrying.`,
        "",
        `${green}⏺${reset} ${bold}cos - ask_user${reset} ${gray}(MCP)${reset}`,
        `  ⎿  ${gray}Question 5 sent${reset}`,
        "",
      ];
    case "notes":
      return [
        ...lines,
        `${green}⏺${reset} ${bold}Bash${reset}(git log --oneline v0.1.0..HEAD)`,
        `  ⎿  ${gray}… +38 lines (ctrl+r to expand)${reset}`,
        "",
        `${green}⏺${reset} ${bold}Write${reset}(CHANGELOG.md)`,
        `  ⎿  Wrote ${bold}46${reset} lines to ${bold}CHANGELOG.md${reset}`,
        "",
        `${white}⏺${reset} Done. The release notes are in CHANGELOG.md under 0.2.0.`,
        "",
      ];
    case "blank":
      return prompt ? lines : [];
  }
}

const spinnerGlyphs = ["·", "✢", "✳", "✶", "✻", "✽"];

export function spinnerLine(verb: string, seconds: number): string {
  const glyph = spinnerGlyphs[seconds % spinnerGlyphs.length];
  const tokens = (0.4 + seconds * 0.03).toFixed(1);
  return `${orange}${glyph}${reset} ${orange}${verb}…${reset} ${gray}(${seconds}s · ↓ ${tokens}k tokens · esc to interrupt)${reset}`;
}

export function renderScreen(spec: ScreenSpec): Screen {
  const width = Math.max(40, Math.min(spec.cols, 110));
  const lines: string[] = [];
  let input: Screen["input"] = null;
  let spinnerRow: number | null = null;

  if (spec.footer.kind === "trust") {
    lines.push(
      ...box(
        [
          `${bold}Do you trust the files in this folder?${reset}`,
          "",
          spec.cwd,
          "",
          "Claude Code may read, write or run files in this folder. Only",
          "trust folders whose code you know.",
          "",
          `${blue}❯${reset} ${blue}1. Yes, proceed${reset}`,
          "  2. No, exit",
        ],
        Math.min(width, 72),
        orange,
      ),
      `   ${gray}Enter to confirm · Esc to exit${reset}`,
    );
  } else {
    lines.push(
      ...box(
        [
          `${orange}✻${reset} Welcome to ${bold}Claude Code${reset}!`,
          "",
          `  ${gray}/help for help, /status for your current setup${reset}`,
          "",
          `  ${gray}cwd: ${spec.cwd}${reset}`,
        ],
        Math.min(width, 60),
        orange,
      ),
      "",
      ...transcript(spec.transcript, spec.prompt),
    );
    switch (spec.footer.kind) {
      case "dialog":
        lines.push(`${green}⏺${reset} ${bold}Bash${reset}(${spec.footer.command})`);
        lines.push(
          ...box(
            [
              `${bold}Bash command${reset}`,
              "",
              `  ${spec.footer.command}`,
              `  ${gray}${spec.footer.description}${reset}`,
              "",
              "Do you want to proceed?",
              `${blue}❯${reset} ${blue}1. Yes${reset}`,
              `  2. Yes, and don't ask again for ${bold}${spec.footer.command.split(" ").slice(0, 2).join(" ")}${reset} commands`,
              `  3. No, and tell Claude what to do differently ${gray}(esc)${reset}`,
            ],
            Math.min(width, 76),
            blue,
          ),
        );
        break;
      case "working":
        spinnerRow = lines.length + 1;
        lines.push(spinnerLine(spec.footer.verb, spec.footer.seconds), "");
        input = { row: lines.length + 2, col: 5, width: width - 6 };
        lines.push(...box([`${gray}>${reset} `], width, gray));
        lines.push(`  ${gray}? for shortcuts${reset}`);
        break;
      case "idle":
        input = { row: lines.length + 2, col: 5, width: width - 6 };
        lines.push(...box([`${gray}>${reset} `], width, gray));
        lines.push(
          `  ${gray}? for shortcuts${reset}${" ".repeat(Math.max(0, width - 40))}${gray}${italic}auto-accept edits off${reset}`,
        );
        break;
      case "exited":
        lines.push(
          `${gray}Resume this session with:${reset}`,
          `claude --resume 5504b543-4897-4751-add5`,
        );
        break;
    }
  }
  // Keep the screen inside the PTY's rows, dropping the oldest lines.
  const overflow = Math.max(0, lines.length - spec.rows);
  const shown = lines.slice(overflow);
  if (input) input = { ...input, row: input.row - overflow };
  if (spinnerRow !== null) spinnerRow -= overflow;
  const cursor = input !== null ? `${ESC}${input.row};${input.col}H${ESC}?25h` : `${ESC}?25l`;
  return {
    data: `${ESC}?2004h${ESC}2J${ESC}H` + shown.join("\r\n") + cursor,
    input,
    spinnerRow,
  };
}

/** Redraws the spinner line in place. */
export function spinnerUpdate(
  row: number,
  verb: string,
  seconds: number,
  back: Screen["input"],
): string {
  const restore = back ? `${ESC}${back.row};${back.col}H` : "";
  return `${ESC}s${ESC}${row};1H${ESC}2K${spinnerLine(verb, seconds)}${ESC}u${restore}`;
}

/** Shows `text` in the input box and leaves the cursor after it. */
export function inputUpdate(input: NonNullable<Screen["input"]>, text: string): string {
  const shown = [...text].slice(-(input.width - 2)).join("");
  const pad = " ".repeat(Math.max(0, input.width - 2 - [...shown].length));
  return `${ESC}${input.row};${input.col}H${shown}${pad}${ESC}${input.row};${input.col + [...shown].length}H`;
}
