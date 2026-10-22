// Environment variables for agents, as in Selene: rows with a mono name, a
// masked value with Show and Hide, Edit and remove (with Undo). Settings
// shows the list for all projects; a project's Environment page shows its
// own, and under it the global ones it inherits, each with Override.
//
// A value is never in the page until you click Show: a hidden one is drawn
// as dots, and the editor's value field is a password field.

import { Eye, EyeOff, Pencil, Plus, RotateCw, Undo2, X } from "lucide-react";
import { type ReactNode, useEffect, useState } from "react";
import { CLAUDE_CONFIG_DIR, type EnvVars, sortedNames } from "../api/env";
import { messageOf } from "../api/errors";
import { useApp, useCanChange, useCos } from "../app/context";
import { accountHint } from "../domain/env";
import { envKey } from "../store/store";
import { cx } from "../ui/cx";
import { ErrorText } from "../ui/Dialog";
import { Button, Card, IconButton, SectionTitle, Spinner } from "../ui/kit";

/** The same length for every value, so a hidden one says nothing about its size. */
const MASK = "••••••••••";

/** How long Undo stays after removing a variable. */
const UNDO_MS = 10_000;

/** One scope's variables: all projects (`projectId` null) or one project's own. */
export function EnvironmentList({
  projectId,
  heading,
}: {
  projectId: number | null;
  heading: ReactNode;
}) {
  const { cos, ui } = useApp();
  const canChange = useCanChange();
  const running = useCos((s) => s.connection === "running");
  const supported = useCos((s) => s.envSupported);
  const own = useCos((s) =>
    projectId === null ? s.globalEnv : (s.projectEnvs[projectId]?.variables ?? null),
  );
  const global = useCos((s) =>
    projectId === null ? null : (s.projectEnvs[projectId]?.global ?? s.globalEnv),
  );
  const loadError = useCos((s) => s.envErrors[envKey(projectId)] ?? null);
  const [shown, setShown] = useState<Record<string, boolean>>({});
  const [removed, setRemoved] = useState<{ name: string; value: string } | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (running) cos.refreshEnv(projectId);
  }, [cos, projectId, running]);

  useEffect(() => {
    if (!removed) return;
    const timer = setTimeout(() => setRemoved(null), UNDO_MS);
    return () => clearTimeout(timer);
  }, [removed]);

  const toggle = (key: string) => setShown((s) => ({ ...s, [key]: !s[key] }));
  const add = (name?: string) =>
    ui.open({
      kind: "envVar",
      projectId,
      name,
      mode: name && global && name in global ? "override" : "add",
    });

  const remove = (name: string, value: string) => {
    setBusy(name);
    setError(null);
    cos.removeEnv(projectId, name).then(
      () => {
        setBusy(null);
        setShown((s) => ({ ...s, [name]: false }));
        setRemoved({ name, value });
      },
      (failure: unknown) => {
        setBusy(null);
        setError(messageOf(failure));
      },
    );
  };

  const undo = () => {
    if (!removed) return;
    const { name, value } = removed;
    setRemoved(null);
    setError(null);
    cos.setEnv(projectId, name, value).catch((failure: unknown) => setError(messageOf(failure)));
  };

  const addButton = (
    <Button size="sm" icon={Plus} disabled={!canChange || !supported} onClick={() => add()}>
      Add environment variable
    </Button>
  );

  let body: ReactNode;
  if (!supported) {
    body = (
      <Card className="px-5 py-4 text-[13px] text-ink-2">
        This cos daemon is older than environment variables. Update cos and restart the daemon.
      </Card>
    );
  } else if (own === null) {
    body = (
      <Card className="flex items-center gap-2 px-5 py-4 text-[13px] text-ink-3">
        {loadError ? (
          <>
            <span className="text-tone-red">{loadError}</span>
            <Button
              size="sm"
              variant="ghost"
              icon={RotateCw}
              className="ml-auto"
              onClick={() => cos.refreshEnv(projectId)}
            >
              Try again
            </Button>
          </>
        ) : running ? (
          <Spinner />
        ) : (
          "Environment variables are kept by the cos daemon, which isn't answering right now."
        )}
      </Card>
    );
  } else {
    const names = sortedNames(Object.keys(own));
    body = (
      <Card>
        {names.length === 0 ? (
          <EmptyList projectId={projectId} onAdd={add} disabled={!canChange} />
        ) : (
          <ul className="divide-y divide-line" aria-label="Variables">
            {names.map((name) => (
              <EnvRow
                key={name}
                name={name}
                value={own[name]!}
                shown={!!shown[name]}
                onToggle={() => toggle(name)}
                note={global && name in global ? "overrides all projects" : null}
              >
                <IconButton
                  icon={Pencil}
                  label={`Edit ${name}`}
                  size="sm"
                  disabled={!canChange || busy === name}
                  onClick={() => ui.open({ kind: "envVar", projectId, name, mode: "edit" })}
                />
                <IconButton
                  icon={X}
                  label={`Remove ${name}`}
                  size="sm"
                  disabled={!canChange || busy === name}
                  onClick={() => remove(name, own[name]!)}
                  className="hover:bg-tone-red/10 hover:text-tone-red"
                />
              </EnvRow>
            ))}
          </ul>
        )}
      </Card>
    );
  }

  return (
    <div>
      <SectionTitle actions={addButton}>{heading}</SectionTitle>
      {body}
      {removed ? (
        <div
          role="status"
          className="mt-2 flex items-center gap-2 rounded-lg bg-subtle py-1 pr-1 pl-3 text-[12.5px] text-ink-2 animate-fade-in"
        >
          <span className="min-w-0 flex-1 truncate">
            Removed <code className="font-mono text-[12px] text-ink">{removed.name}</code>.
          </span>
          <Button size="sm" variant="ghost" icon={Undo2} disabled={!canChange} onClick={undo}>
            Undo
          </Button>
        </div>
      ) : null}
      {error ? (
        <div className="mt-2">
          <ErrorText>{error}</ErrorText>
        </div>
      ) : null}
      {global && own !== null ? (
        <Inherited
          global={global}
          own={own}
          shown={shown}
          onToggle={toggle}
          onOverride={(name) => ui.open({ kind: "envVar", projectId, name, mode: "override" })}
          disabled={!canChange}
        />
      ) : null}
    </div>
  );
}

/** A variable: mono name, masked value, Show/Hide, then the row's own actions. */
function EnvRow({
  name,
  value,
  shown,
  onToggle,
  note,
  muted,
  children,
}: {
  name: string;
  value: string;
  shown: boolean;
  onToggle: () => void;
  note?: string | null;
  muted?: boolean;
  children: ReactNode;
}) {
  return (
    <li aria-label={name} className="group flex min-h-11 items-center gap-3 px-4 py-2">
      <div className="flex w-[38%] max-w-[260px] min-w-0 shrink-0 flex-col">
        <code
          title={name}
          className={cx(
            "truncate font-mono text-[12.5px] font-medium",
            muted ? "text-ink-2" : "text-ink",
          )}
        >
          {name}
        </code>
        {note ? <span className="text-[11.5px] text-ink-3">{note}</span> : null}
      </div>
      <div className="min-w-0 flex-1">
        {shown ? (
          <code className="font-mono text-[12.5px] break-all text-ink-2 select-text">
            {value === "" ? <span className="text-ink-3 italic">empty</span> : value}
          </code>
        ) : (
          <span
            aria-label="Hidden value"
            className="text-[13px] leading-none tracking-[0.08em] text-ink-3 select-none"
          >
            {MASK}
          </span>
        )}
      </div>
      <div className="flex shrink-0 items-center gap-0.5">
        <button
          type="button"
          aria-label={`${shown ? "Hide" : "Show"} ${name}`}
          aria-pressed={shown}
          onClick={onToggle}
          className="flex h-6 items-center gap-1 rounded-md px-1.5 text-[12px] text-ink-3 hover:bg-subtle-2/70 hover:text-ink"
        >
          {shown ? (
            <EyeOff className="size-3.5" strokeWidth={1.8} aria-hidden />
          ) : (
            <Eye className="size-3.5" strokeWidth={1.8} aria-hidden />
          )}
          {shown ? "Hide" : "Show"}
        </button>
        {children}
      </div>
    </li>
  );
}

function EmptyList({
  projectId,
  onAdd,
  disabled,
}: {
  projectId: number | null;
  onAdd: (name: string) => void;
  disabled: boolean;
}) {
  return (
    <div className="flex flex-col items-start gap-3 px-4 py-4">
      <p className="text-[13px] text-ink-2">
        {projectId === null
          ? "No variables for all projects yet."
          : "This project sets no variables of its own yet."}
      </p>
      <button
        type="button"
        disabled={disabled}
        onClick={() => onAdd(CLAUDE_CONFIG_DIR)}
        className="group flex items-center gap-2 rounded-full border border-dashed border-line-strong py-1 pr-3 pl-1.5 text-left text-[12.5px] text-ink-2 transition-colors hover:border-accent/50 hover:bg-accent/6 hover:text-ink disabled:opacity-45"
      >
        <span className="flex size-5 items-center justify-center rounded-full bg-subtle text-ink-3 group-hover:bg-accent/15 group-hover:text-accent">
          <Plus className="size-3" strokeWidth={2.2} aria-hidden />
        </span>
        <code className="font-mono text-[12px] font-medium text-ink">{CLAUDE_CONFIG_DIR}</code>
        <span className="text-ink-3">{accountHint(projectId)}</span>
      </button>
    </div>
  );
}

/** Global variables a project doesn't override, muted, each with Override. */
function Inherited({
  global,
  own,
  shown,
  onToggle,
  onOverride,
  disabled,
}: {
  global: EnvVars;
  own: EnvVars;
  shown: Record<string, boolean>;
  onToggle: (key: string) => void;
  onOverride: (name: string) => void;
  disabled: boolean;
}) {
  const { ui } = useApp();
  const names = sortedNames(Object.keys(global).filter((name) => !(name in own)));
  if (names.length === 0) return null;
  return (
    <section aria-label="Inherited from all projects" className="mt-8">
      <SectionTitle
        actions={
          <Button size="sm" variant="ghost" onClick={() => ui.navigate({ kind: "settings" })}>
            Edit in Settings
          </Button>
        }
      >
        <span className="text-ink-3">Inherited from all projects</span>
      </SectionTitle>
      <ul
        className="divide-y divide-line rounded-xl border border-dashed border-line-strong"
        aria-label="Inherited variables"
      >
        {names.map((name) => (
          <EnvRow
            key={name}
            name={name}
            value={global[name]!}
            shown={!!shown[`global:${name}`]}
            onToggle={() => onToggle(`global:${name}`)}
            muted
          >
            <Button
              size="sm"
              variant="ghost"
              aria-label={`Override ${name}`}
              disabled={disabled}
              onClick={() => onOverride(name)}
              className="ml-1"
            >
              Override
            </Button>
          </EnvRow>
        ))}
      </ul>
    </section>
  );
}
