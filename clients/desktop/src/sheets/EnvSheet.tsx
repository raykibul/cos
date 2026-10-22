// Adding or changing an environment variable, for all projects or one. The
// name is checked here; names cos keeps to itself are refused by the daemon,
// whose reason shows inline. The value is a password field until you show it.

import { Eye, EyeOff } from "lucide-react";
import { useId, useLayoutEffect, useRef, useState } from "react";
import { CLAUDE_CONFIG_DIR, nameProblem, valueProblem } from "../api/env";
import { messageOf } from "../api/errors";
import { useApp, useCanChange, useCos } from "../app/context";
import type { DialogState } from "../app/ui";
import { accountHint } from "../domain/env";
import { project as projectOf } from "../store/selectors";
import { cx } from "../ui/cx";
import { Dialog, ErrorText } from "../ui/Dialog";
import { Button, Field, inputClass } from "../ui/kit";

type EnvDialog = Extract<DialogState, { kind: "envVar" }>;

export function EnvVarSheet({ dialog }: { dialog: EnvDialog }) {
  const { cos, ui } = useApp();
  const canChange = useCanChange();
  const { projectId } = dialog;
  const mode = dialog.mode ?? "add";
  const fixed = mode !== "add" && !!dialog.name;
  const projectName = useCos((s) =>
    projectId === null ? null : (projectOf(s, projectId)?.name ?? null),
  );
  const own = useCos((s) =>
    projectId === null ? s.globalEnv : (s.projectEnvs[projectId]?.variables ?? null),
  );
  const [name, setName] = useState(dialog.name ?? "");
  // Untouched, an edit shows the current value, even if it loads after the sheet opens.
  const [draft, setDraft] = useState<string | null>(null);
  const [shown, setShown] = useState(false);
  const [tried, setTried] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const nameId = useId();
  const valueId = useId();

  const trimmed = name.trim();
  const initial = mode === "edit" ? (own?.[trimmed] ?? "") : "";
  const value = draft ?? initial;
  // The value field is uncontrolled: React mirrors a controlled input's
  // value into its `value` attribute, which would put the secret in the
  // page's markup. Only the property holds it.
  const valueInput = useRef<HTMLInputElement>(null);
  useLayoutEffect(() => {
    const el = valueInput.current;
    if (el && draft === null && el.value !== initial) el.value = initial;
  }, [draft, initial]);
  const nameError = fixed ? null : nameProblem(trimmed);
  const valueError = valueProblem(value);
  const replaces = mode === "add" && !!own && trimmed in own;
  const showNameError = nameError !== null && (tried || trimmed !== "");

  const submit = () => {
    setTried(true);
    if (nameError || valueError || busy) return;
    setBusy(true);
    setError(null);
    cos.setEnv(projectId, trimmed, value).then(
      () => {
        setBusy(false);
        ui.close();
      },
      (failure: unknown) => {
        setBusy(false);
        setError(messageOf(failure));
      },
    );
  };

  const title =
    mode === "edit" ? (
      <>
        Edit <span className="font-mono text-[17px]">{trimmed}</span>
      </>
    ) : mode === "override" ? (
      <>
        Override <span className="font-mono text-[17px]">{trimmed}</span>
      </>
    ) : (
      "Add environment variable"
    );
  const scope =
    projectId === null
      ? "For every project's agents, unless a project sets its own."
      : mode === "override"
        ? `Gives ${projectName ?? "this project"} its own value instead of the one for all projects.`
        : `For the agents in ${projectName ?? "this project"}.`;
  const account = trimmed === CLAUDE_CONFIG_DIR;

  return (
    <Dialog
      title={title}
      description={`${scope} Running agents keep their current values until they restart.`}
      onClose={ui.close}
      onSubmit={submit}
      footer={
        <>
          <Button variant="ghost" onClick={ui.close}>
            Cancel
          </Button>
          <Button
            type="submit"
            variant="primary"
            busy={busy}
            disabled={!canChange || (tried && (!!nameError || !!valueError))}
          >
            {mode === "add"
              ? replaces
                ? "Replace"
                : "Add variable"
              : mode === "edit"
                ? "Save"
                : "Override"}
          </Button>
        </>
      }
    >
      <Field
        label="Name"
        htmlFor={nameId}
        hint={
          showNameError ? (
            <span className="text-tone-red" role="alert">
              {nameError}
            </span>
          ) : replaces ? (
            `${trimmed} is already set here; this replaces its value.`
          ) : fixed ? undefined : (
            "Letters, digits and _, not starting with a digit."
          )
        }
      >
        <input
          id={nameId}
          data-autofocus={fixed ? undefined : true}
          readOnly={fixed}
          aria-invalid={showNameError || undefined}
          className={cx(
            // A fixed name reads as a label: muted, no focus ring.
            fixed
              ? inputClass.replace("bg-surface", "bg-subtle border-transparent") +
                  " cursor-default text-ink-2 focus:border-transparent focus:ring-0"
              : inputClass,
            "font-mono text-[13px]",
            showNameError && "border-tone-red/60 focus:border-tone-red/60 focus:ring-tone-red/15",
          )}
          value={name}
          placeholder={CLAUDE_CONFIG_DIR}
          autoComplete="off"
          autoCorrect="off"
          autoCapitalize="off"
          spellCheck={false}
          maxLength={256}
          onChange={(event) => {
            setName(event.target.value);
            setError(null);
          }}
        />
      </Field>
      <Field
        label="Value"
        htmlFor={valueId}
        hint={
          valueError ? (
            <span className="text-tone-red" role="alert">
              {valueError}
            </span>
          ) : account ? (
            `${accountHint(projectId)}. A leading ~/ is your home folder.`
          ) : (
            "A leading ~/ is your home folder."
          )
        }
      >
        <div className="relative">
          <input
            id={valueId}
            data-autofocus={fixed ? true : undefined}
            type={shown ? "text" : "password"}
            className={cx(inputClass, "pr-10 font-mono text-[13px]")}
            ref={valueInput}
            placeholder={account ? "~/.claude-personal" : "Value"}
            autoComplete="off"
            autoCorrect="off"
            autoCapitalize="off"
            spellCheck={false}
            onChange={(event) => {
              setDraft(event.target.value);
              setError(null);
            }}
          />
          <button
            type="button"
            aria-label={shown ? "Hide value" : "Show value"}
            aria-pressed={shown}
            title={shown ? "Hide value" : "Show value"}
            onClick={() => setShown(!shown)}
            className="absolute top-1/2 right-1.5 flex size-7 -translate-y-1/2 items-center justify-center rounded-md text-ink-3 hover:bg-subtle hover:text-ink"
          >
            {shown ? (
              <EyeOff className="size-4" strokeWidth={1.8} aria-hidden />
            ) : (
              <Eye className="size-4" strokeWidth={1.8} aria-hidden />
            )}
          </button>
        </div>
      </Field>
      <ErrorText>{error}</ErrorText>
    </Dialog>
  );
}
