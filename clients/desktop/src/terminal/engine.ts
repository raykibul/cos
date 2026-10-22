// The one file that knows the terminal emulator: ghostty-web, Ghostty's VT
// engine in WebAssembly with an xterm.js-like API. Swapping to @xterm/xterm
// means rewriting this file only; everything else uses `TerminalEngine`.

import { FitAddon, Ghostty, Terminal } from "ghostty-web";
// Emitted into dist/assets as its own file, so it loads from Tauri's custom
// protocol like any other asset.
import wasmUrl from "ghostty-web/ghostty-vt.wasm?url";
import type { TerminalTheme } from "./theme";
import { type WheelState, wheelData } from "./wheel";

export interface Disposable {
  dispose(): void;
}

export interface TerminalEngine {
  open(parent: HTMLElement): void;
  /** Raw PTY bytes, split anywhere. */
  write(bytes: Uint8Array): void;
  /** Back to a blank screen with default modes, before a snapshot. */
  reset(): void;
  resize(cols: number, rows: number): void;
  /** How many cells fit the parent, or null before layout. */
  fitSize(): { cols: number; rows: number } | null;
  focus(): void;
  /** What to send to the PTY: keys, pastes (bracketed when the app asks), mouse reports. */
  onData(listener: (data: string) => void): Disposable;
  dispose(): void;
}

export interface EngineOptions {
  theme: TerminalTheme;
  fontFamily: string;
  fontSize: number;
}

let compiled: Promise<WebAssembly.Module> | null = null;

/** Compiles the bundled WASM once. */
function compileEngine(): Promise<WebAssembly.Module> {
  compiled ??= fetch(wasmUrl)
    .then((response) => {
      if (!response.ok) throw new Error(`fetching ${wasmUrl}: ${response.status}`);
      return response.arrayBuffer();
    })
    .then((bytes) => WebAssembly.compile(bytes));
  compiled.catch(() => {
    compiled = null;
  });
  return compiled;
}

/**
 * A fresh WASM instance for one terminal. Terminals must not share one:
 * with ghostty-web 0.4.0, a terminal opened after another one was disposed
 * on the same instance draws the old terminal's cells, scrambled, with a
 * full-height cursor.
 */
export async function loadEngine(): Promise<Ghostty> {
  try {
    const module = await compileEngine();
    let instance: WebAssembly.Instance | null = null;
    instance = await WebAssembly.instantiate(module, {
      env: {
        // The same logging hook ghostty-web passes.
        log: (pointer: number, length: number) => {
          const memory = instance?.exports.memory as WebAssembly.Memory | undefined;
          if (!memory) return;
          const text = new TextDecoder().decode(new Uint8Array(memory.buffer, pointer, length));
          console.log("[ghostty-vt]", text);
        },
      },
    });
    return new Ghostty(instance);
  } catch (error: unknown) {
    // Without a path, ghostty-web reads the copy inlined in its JS as a data:
    // URL; each call still makes its own instance.
    console.warn("ghostty-web: loading the bundled WASM failed, trying the inlined copy", error);
    return Ghostty.load();
  }
}

const isMac = typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform);

async function copyText(text: string): Promise<void> {
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    const area = document.createElement("textarea");
    area.value = text;
    area.style.position = "fixed";
    area.style.opacity = "0";
    document.body.appendChild(area);
    area.select();
    document.execCommand("copy");
    area.remove();
  }
}

class GhosttyEngine implements TerminalEngine {
  private readonly term: Terminal;
  private readonly fit = new FitAddon();
  private parent: HTMLElement | null = null;
  private readonly cleanups: (() => void)[] = [];
  /** onData listeners, so input made here (the wheel) reaches them too. */
  private readonly listeners = new Set<(data: string) => void>();

  constructor(ghostty: Ghostty, options: EngineOptions) {
    this.term = new Terminal({
      ghostty,
      fontFamily: options.fontFamily,
      fontSize: options.fontSize,
      theme: options.theme,
      cursorBlink: false,
      scrollback: 5000,
      smoothScrollDuration: 0,
    });
    this.term.loadAddon(this.fit);
  }

  open(parent: HTMLElement): void {
    this.parent = parent;
    this.term.open(parent);
    // Cmd-C copies the selection, Cmd-A selects everything, and neither
    // reaches Claude as a keystroke. ghostty-web's handler returns true for
    // "handled" (xterm.js uses false).
    this.term.attachCustomKeyEventHandler((event) => {
      if (event.type !== "keydown") return false;
      const command = isMac ? event.metaKey && !event.ctrlKey : event.ctrlKey && event.shiftKey;
      if (!command || event.altKey) return false;
      const key = event.key.toLowerCase();
      if (key === "c") {
        if (this.term.hasSelection()) void copyText(this.term.getSelection());
        return true;
      }
      if (key === "a") {
        this.term.selectAll();
        return true;
      }
      // Cmd-V: let the browser fire its paste event, handled below.
      return false;
    });
    // Pastes go through `paste()`, which brackets them when the app turned
    // bracketed paste on. Caught before ghostty-web's own listeners so a
    // paste is sent once.
    const onPaste = (event: ClipboardEvent) => {
      const text = event.clipboardData?.getData("text/plain");
      event.preventDefault();
      event.stopImmediatePropagation();
      if (text) this.term.paste(text);
    };
    parent.addEventListener("paste", onPaste, { capture: true });
    this.cleanups.push(() => parent.removeEventListener("paste", onPaste, { capture: true }));

    // On the alternate screen, the wheel becomes mouse reports (or Page
    // Up/Down) instead of ghostty-web's arrow keys; see wheel.ts.
    const wheel: WheelState = { carry: 0 };
    this.term.attachCustomWheelEventHandler((event) => {
      if (this.term.buffer.active.type !== "alternate") {
        wheel.carry = 0;
        return false;
      }
      const rect = parent.getBoundingClientRect();
      const data = wheelData(
        {
          deltaY: event.deltaY,
          deltaMode: event.deltaMode,
          x: event.clientX - rect.left,
          y: event.clientY - rect.top,
          width: rect.width,
          height: rect.height,
          cols: this.term.cols,
          rows: this.term.rows,
          mouseTracking: [1000, 1002, 1003].some((mode) => this.term.getMode(mode)),
          sgr: this.term.getMode(1006),
        },
        wheel,
      );
      if (data) for (const listener of this.listeners) listener(data);
      return true;
    });
  }

  write(bytes: Uint8Array): void {
    this.term.write(bytes);
  }

  reset(): void {
    this.term.reset();
  }

  resize(cols: number, rows: number): void {
    if (cols > 0 && rows > 0) this.term.resize(cols, rows);
  }

  fitSize(): { cols: number; rows: number } | null {
    if (!this.parent || this.parent.clientWidth === 0 || this.parent.clientHeight === 0)
      return null;
    const size = this.fit.proposeDimensions();
    if (!size || size.cols <= 0 || size.rows <= 0) return null;
    return { cols: Math.floor(size.cols), rows: Math.floor(size.rows) };
  }

  focus(): void {
    this.term.focus();
  }

  onData(listener: (data: string) => void): Disposable {
    this.listeners.add(listener);
    const own = this.term.onData(listener);
    return {
      dispose: () => {
        this.listeners.delete(listener);
        own.dispose();
      },
    };
  }

  dispose(): void {
    for (const cleanup of this.cleanups) cleanup();
    this.term.dispose();
  }
}

export async function createEngine(options: EngineOptions): Promise<TerminalEngine> {
  const ghostty = await loadEngine();
  return new GhosttyEngine(ghostty, options);
}
