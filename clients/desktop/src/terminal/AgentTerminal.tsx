import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { useApp } from "../app/context";
import { cx } from "../ui/cx";
import { Spinner } from "../ui/kit";
import { useScheme } from "../ui/scheme";
import { type TerminalEngine, createEngine } from "./engine";
import { shellBackend, terminalBackend } from "./backend";
import { type TerminalPhase, TerminalSession } from "./session";
import { darkTheme, lightTheme, terminalFont, terminalFontSize } from "./theme";

/**
 * An agent's live terminal, or a shell's (`kind="shell"`, with the shell's id
 * as `agentId`). It takes keys and focus like a terminal app: typing goes to
 * Claude or the shell, Cmd-C copies, Cmd-V pastes. The emulator is rebuilt
 * with a fresh snapshot when the colour scheme changes.
 */
export function AgentTerminal({
  agentId,
  kind = "agent",
  alive,
  className,
  autoFocus = true,
}: {
  agentId: number;
  kind?: "agent" | "shell";
  /** Whether Claude runs. Going from exited back to alive (a resume) reconnects. */
  alive: boolean;
  className?: string;
  autoFocus?: boolean;
}) {
  const { cos } = useApp();
  const host = useRef<HTMLDivElement>(null);
  const engineRef = useRef<TerminalEngine | null>(null);
  const scheme = useScheme();
  const [loadError, setLoadError] = useState<string | null>(null);
  const session = useMemo(
    () => new TerminalSession(agentId, kind === "shell" ? shellBackend(cos) : terminalBackend(cos)),
    [agentId, kind, cos],
  );
  const phase = useSyncExternalStore(session.subscribe, session.getPhase);

  useEffect(() => {
    const parent = host.current;
    if (!parent) return;
    let disposed = false;
    let engine: TerminalEngine | null = null;
    let observer: ResizeObserver | null = null;
    let data: { dispose(): void } | null = null;
    let frame = 0;

    createEngine({
      theme: scheme === "dark" ? darkTheme : lightTheme,
      fontFamily: terminalFont,
      fontSize: terminalFontSize,
    }).then(
      (created) => {
        if (disposed) {
          created.dispose();
          return;
        }
        engine = created;
        engineRef.current = created;
        created.open(parent);
        session.sink = {
          reset: (size) => {
            created.reset();
            created.resize(size.cols, size.rows);
          },
          write: (bytes) => created.write(bytes),
          resize: (size) => created.resize(size.cols, size.rows),
        };
        data = created.onData((text) => session.send(text));
        const report = () => {
          cancelAnimationFrame(frame);
          frame = requestAnimationFrame(() => {
            const size = created.fitSize();
            if (size) session.viewSizeChanged(size);
          });
        };
        observer = new ResizeObserver(report);
        observer.observe(parent);
        report();
        session.start();
        if (autoFocus) created.focus();
      },
      (error: unknown) => {
        if (!disposed) setLoadError(error instanceof Error ? error.message : String(error));
      },
    );

    return () => {
      disposed = true;
      cancelAnimationFrame(frame);
      session.stop();
      session.sink = null;
      observer?.disconnect();
      data?.dispose();
      engine?.dispose();
      engineRef.current = null;
    };
  }, [session, scheme, autoFocus]);

  // A resume: the old stream ended with the process, so open a new one.
  const wasAlive = useRef(alive);
  useEffect(() => {
    if (alive && !wasAlive.current && session.phase.kind === "exited") session.restart();
    wasAlive.current = alive;
  }, [alive, session]);

  return (
    <div
      className={cx(
        "relative min-h-0 overflow-hidden rounded-xl border border-line bg-[var(--c-terminal)]",
        className,
      )}
    >
      <div className="absolute inset-x-3 inset-y-2.5">
        <div
          ref={host}
          className="terminal-host"
          onMouseDown={() => engineRef.current?.focus()}
          data-testid="terminal"
        />
      </div>
      <PhaseOverlay phase={phase} loadError={loadError} kind={kind} />
    </div>
  );
}

function PhaseOverlay({
  phase,
  loadError,
  kind,
}: {
  phase: TerminalPhase;
  loadError: string | null;
  kind: "agent" | "shell";
}) {
  if (loadError) {
    return <Notice tone="error">The terminal couldn't load: {loadError}</Notice>;
  }
  switch (phase.kind) {
    case "connecting":
      return (
        <div className="pointer-events-none absolute inset-0 flex items-center justify-center">
          <Spinner />
        </div>
      );
    case "reconnecting":
      return <Notice>Reconnecting… {phase.reason}</Notice>;
    case "failed":
      return <Notice tone="error">{phase.message}</Notice>;
    case "exited":
      if (kind === "shell") {
        return (
          <Notice>
            The shell exited{phase.code !== null ? ` with code ${phase.code}` : ""}. Ctrl-J opens
            another.
          </Notice>
        );
      }
      return (
        <Notice>
          Claude exited{phase.code !== null ? ` with code ${phase.code}` : ""}. Resume it to go on.
        </Notice>
      );
    case "live":
      return null;
  }
}

function Notice({ children, tone }: { children: React.ReactNode; tone?: "error" }) {
  return (
    <div className="pointer-events-none absolute inset-x-0 bottom-3 flex justify-center px-4">
      <div
        role="status"
        className={cx(
          "rounded-full border border-line bg-raised px-3.5 py-1.5 text-[12.5px] shadow-pop animate-fade-in",
          tone === "error" ? "text-tone-red" : "text-ink-2",
        )}
      >
        {children}
      </div>
    </div>
  );
}
