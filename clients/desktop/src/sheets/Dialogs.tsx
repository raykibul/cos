// Every sheet the app opens, chosen by the UI store's `dialog`. Each one
// shows the daemon's error as-is and stays open until the action succeeds.

import { useState } from "react";
import { messageOf } from "../api/errors";
import { projectSlug, workspaceName } from "../api/models";
import { useApp, useCanChange, useCos, useUi } from "../app/context";
import type { DialogState } from "../app/ui";
import { presetExplanation, presetTitle, presets } from "../domain/labels";
import { type TaskActionKind, needsReason, targetState } from "../domain/tasks";
import { Composer } from "../screens/Composer";
import { toNewAgent } from "../screens/composerValue";
import {
  agent as agentOf,
  project as projectOf,
  run as runOf,
  workspace as workspaceOf,
} from "../store/selectors";
import { cx } from "../ui/cx";
import { Dialog, ErrorText } from "../ui/Dialog";
import { Button, Field, inputClass } from "../ui/kit";
import { AddHostSheet, RemoveHostSheet } from "./HostSheets";

export function Dialogs() {
  const dialog = useUi((s) => s.dialog);
  if (!dialog) return null;
  return <DialogFor dialog={dialog} />;
}

function DialogFor({ dialog }: { dialog: DialogState }) {
  switch (dialog.kind) {
    case "addProject":
      return <AddProject />;
    case "removeProject":
      return <RemoveProject projectId={dialog.projectId} />;
    case "newWorkspace":
      return <NewWorkspace projectId={dialog.projectId} />;
    case "renameWorkspace":
      return <RenameWorkspace workspaceId={dialog.workspaceId} />;
    case "removeWorkspace":
      return <RemoveWorkspace workspaceId={dialog.workspaceId} />;
    case "newAgent":
      return <NewAgentSheet workspaceId={dialog.workspaceId} />;
    case "removeAgent":
      return <RemoveAgent agentId={dialog.agentId} />;
    case "newRun":
      return <NewRunSheet workspaceId={dialog.workspaceId} />;
    case "cancelRun":
      return <CancelRun runId={dialog.runId} />;
    case "taskAction":
      return <TaskAction runId={dialog.runId} taskId={dialog.taskId} action={dialog.action} />;
    case "limits":
      return <LimitsSheet runId={dialog.runId} />;
    case "addHost":
      return <AddHostSheet />;
    case "removeHost":
      return <RemoveHostSheet hostId={dialog.hostId} />;
  }
}

/** Busy and error state for a sheet's one action; closes on success. */
function useAction() {
  const { ui } = useApp();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const run = (action: () => Promise<unknown>) => {
    if (busy) return;
    setBusy(true);
    setError(null);
    action().then(
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
  return { busy, error, run, close: ui.close };
}

function Footer({
  busy,
  label,
  disabled,
  danger,
  onCancel,
}: {
  busy: boolean;
  label: string;
  disabled?: boolean;
  danger?: boolean;
  onCancel: () => void;
}) {
  const canChange = useCanChange();
  return (
    <>
      <Button variant="ghost" onClick={onCancel}>
        Cancel
      </Button>
      <Button
        type="submit"
        variant={danger ? "destructive" : "primary"}
        busy={busy}
        disabled={disabled || !canChange}
      >
        {label}
      </Button>
    </>
  );
}

// MARK: Projects

function AddProject() {
  const { cos, ui } = useApp();
  const { busy, error, run, close } = useAction();
  const [path, setPath] = useState("");
  const trimmed = path.trim();
  const submit = () => {
    if (!trimmed) return;
    run(async () => {
      const project = await cos.addProject(trimmed);
      // A new project opens on its Chief of Staff, ready for a first message.
      ui.navigate({ kind: "chief", id: project.id });
      ui.setExpanded(project.id, true);
    });
  };
  return (
    <Dialog
      title="Add a project"
      description="A git repo on this Mac. Agents never work in it directly; each workspace is a worktree of it."
      onClose={close}
      onSubmit={submit}
      footer={<Footer busy={busy} label="Add project" disabled={!trimmed} onCancel={close} />}
    >
      <Field
        label="Folder"
        htmlFor="project-path"
        hint="The absolute path. A subfolder or worktree registers its main checkout."
      >
        <input
          id="project-path"
          className={cx(inputClass, "font-mono text-[13px]")}
          placeholder="/Users/you/code/my-repo"
          value={path}
          spellCheck={false}
          autoCapitalize="off"
          autoCorrect="off"
          onChange={(event) => setPath(event.target.value)}
        />
      </Field>
      <ErrorText>{error}</ErrorText>
    </Dialog>
  );
}

function RemoveProject({ projectId }: { projectId: number }) {
  const { cos, ui } = useApp();
  const project = useCos((s) => projectOf(s, projectId));
  const { busy, error, run, close } = useAction();
  if (!project) return null;
  const blocked = project.workspaces.length > 0;
  return (
    <Dialog
      width="sm"
      title={`Remove ${project.name}?`}
      description={
        blocked
          ? "Remove its workspaces first. A project with workspaces can't be removed."
          : "cos forgets the project. Your repo isn't touched."
      }
      onClose={close}
      onSubmit={() =>
        run(async () => {
          await cos.removeProject(projectId);
          ui.navigate({ kind: "waiting" });
        })
      }
      footer={<Footer busy={busy} label="Remove" danger disabled={blocked} onCancel={close} />}
    >
      <ErrorText>{error}</ErrorText>
    </Dialog>
  );
}

// MARK: Workspaces

function NewWorkspace({ projectId }: { projectId: number }) {
  const { cos, ui } = useApp();
  const project = useCos((s) => projectOf(s, projectId));
  const { busy, error, run, close } = useAction();
  const [base, setBase] = useState<"origin" | "head">(
    project?.hasOrigin === false ? "head" : "origin",
  );
  const [title, setTitle] = useState("");
  if (!project) return null;
  const options = [
    {
      value: "origin" as const,
      title: `origin/${project.baseBranch}`,
      detail: "Fetches first and starts from exactly that commit.",
      disabled: !project.hasOrigin,
    },
    {
      value: "head" as const,
      title: "Your current HEAD",
      detail: "Includes commits you haven't pushed. Uncommitted changes aren't copied.",
      disabled: false,
    },
  ];
  return (
    <Dialog
      title={`New workspace in ${project.name}`}
      description={`A worktree on its own branch, ${projectSlug(project.name)}/<city>. cos picks the city.`}
      onClose={close}
      onSubmit={() =>
        run(async () => {
          const workspace = await cos.createWorkspace(projectId, base, title);
          ui.navigate({ kind: "workspace", id: workspace.id });
          ui.setExpanded(projectId, true);
        })
      }
      footer={<Footer busy={busy} label="Create workspace" onCancel={close} />}
    >
      <Field
        label="Title"
        htmlFor="workspace-title"
        hint="Optional. Without one, the workspace goes by its city."
      >
        <input
          id="workspace-title"
          className={inputClass}
          placeholder="Fix the login flow"
          value={title}
          onChange={(event) => setTitle(event.target.value)}
        />
      </Field>
      <fieldset className="flex flex-col gap-2">
        <legend className="mb-1.5 text-[12.5px] font-medium text-ink-2">Start from</legend>
        {options.map((option) => (
          <Choice
            key={option.value}
            name="base"
            checked={base === option.value}
            disabled={option.disabled}
            onChange={() => setBase(option.value)}
            title={option.title}
            detail={option.disabled ? "This repo has no origin remote." : option.detail}
            mono
          />
        ))}
      </fieldset>
      <ErrorText>{error}</ErrorText>
    </Dialog>
  );
}

function Choice({
  name,
  checked,
  disabled,
  onChange,
  title,
  detail,
  mono,
}: {
  name: string;
  checked: boolean;
  disabled?: boolean;
  onChange: () => void;
  title: string;
  detail: string;
  mono?: boolean;
}) {
  return (
    <label
      className={cx(
        "flex gap-3 rounded-xl border px-3.5 py-2.5 transition-colors",
        checked ? "border-accent/50 bg-accent/6" : "border-line-strong hover:bg-subtle/60",
        disabled && "pointer-events-none opacity-50",
      )}
    >
      <input
        type="radio"
        name={name}
        checked={checked}
        disabled={disabled}
        onChange={onChange}
        className="mt-1 accent-[var(--c-accent)]"
      />
      <span className="min-w-0">
        <span
          className={cx(
            "block text-[13.5px] font-medium text-ink",
            mono && "font-mono text-[13px]",
          )}
        >
          {title}
        </span>
        <span className="block text-[12.5px] leading-snug text-ink-2">{detail}</span>
      </span>
    </label>
  );
}

function RenameWorkspace({ workspaceId }: { workspaceId: number }) {
  const { cos } = useApp();
  const workspace = useCos((s) => workspaceOf(s, workspaceId));
  const { busy, error, run, close } = useAction();
  const [title, setTitle] = useState(workspace?.title ?? "");
  if (!workspace) return null;
  const trimmed = title.trim();
  return (
    <Dialog
      width="sm"
      title="Rename workspace"
      description={`Only the title changes. The folder and the branch stay ${workspace.name}.`}
      onClose={close}
      onSubmit={() => trimmed && run(() => cos.renameWorkspace(workspace, trimmed))}
      footer={<Footer busy={busy} label="Rename" disabled={!trimmed} onCancel={close} />}
    >
      <Field label="Title" htmlFor="workspace-rename">
        <input
          id="workspace-rename"
          className={inputClass}
          value={title}
          onChange={(event) => setTitle(event.target.value)}
        />
      </Field>
      <ErrorText>{error}</ErrorText>
    </Dialog>
  );
}

function RemoveWorkspace({ workspaceId }: { workspaceId: number }) {
  const { cos, ui } = useApp();
  const workspace = useCos((s) => workspaceOf(s, workspaceId));
  const { busy, error, run, close } = useAction();
  if (!workspace) return null;
  return (
    <Dialog
      width="sm"
      title={`Remove ${workspaceName(workspace)}?`}
      description={
        <>
          The folder is deleted only if it has no uncommitted changes. The branch{" "}
          <code className="font-mono text-[12.5px]">{workspace.branch}</code> is kept, so no commit
          is lost.
        </>
      }
      onClose={close}
      onSubmit={() =>
        run(async () => {
          await cos.removeWorkspace(workspace);
          ui.navigate({ kind: "project", id: workspace.projectId });
        })
      }
      footer={<Footer busy={busy} label="Remove" danger onCancel={close} />}
    >
      <ErrorText>{error}</ErrorText>
    </Dialog>
  );
}

// MARK: Agents

function NewAgentSheet({ workspaceId }: { workspaceId: number }) {
  const { cos, ui } = useApp();
  const workspace = useCos((s) => workspaceOf(s, workspaceId));
  const canChange = useCanChange();
  const [title, setTitle] = useState("");
  const [error, setError] = useState<string | null>(null);
  if (!workspace) return null;
  return (
    <Dialog
      width="lg"
      title={`New agent in ${workspaceName(workspace)}`}
      description="Claude Code starts in the workspace folder. Leave the prompt empty to start it idle."
      onClose={ui.close}
    >
      <Field label="Title" htmlFor="agent-title" hint="Optional. Taken from the prompt otherwise.">
        <input
          id="agent-title"
          className={inputClass}
          value={title}
          placeholder="Login fix"
          onChange={(event) => setTitle(event.target.value)}
        />
      </Field>
      <Composer
        autoFocus
        disabled={!canChange}
        onSend={(value) =>
          cos.createAgent(workspaceId, toNewAgent(value, title)).then(
            (agent) => {
              ui.close();
              ui.navigate({ kind: "agent", id: agent.id });
            },
            (failure: unknown) => {
              setError(messageOf(failure));
              throw failure;
            },
          )
        }
      />
      <ErrorText>{error}</ErrorText>
    </Dialog>
  );
}

function RemoveAgent({ agentId }: { agentId: number }) {
  const { cos, ui } = useApp();
  const agent = useCos((s) => agentOf(s, agentId));
  const { busy, error, run, close } = useAction();
  if (!agent) return null;
  return (
    <Dialog
      width="sm"
      title={`Remove ${agent.title}?`}
      description="cos forgets the agent and its terminal. The workspace and Claude's own session history stay."
      onClose={close}
      onSubmit={() =>
        run(async () => {
          await cos.removeAgent(agentId);
          ui.navigate(
            agent.workspaceId !== null
              ? { kind: "workspace", id: agent.workspaceId }
              : { kind: "waiting" },
          );
        })
      }
      footer={<Footer busy={busy} label="Remove" danger onCancel={close} />}
    >
      <ErrorText>{error}</ErrorText>
    </Dialog>
  );
}

// MARK: Runs

function PresetPicker({ value, onChange }: { value: string; onChange: (preset: string) => void }) {
  return (
    <fieldset className="flex flex-col gap-2">
      <legend className="mb-1.5 text-[12.5px] font-medium text-ink-2">
        What the chief may approve
      </legend>
      {presets.map((preset) => (
        <Choice
          key={preset}
          name="preset"
          checked={value === preset}
          onChange={() => onChange(preset)}
          title={presetTitle(preset)}
          detail={presetExplanation(preset)}
        />
      ))}
    </fieldset>
  );
}

function CapField({ value, onChange }: { value: string; onChange: (cap: string) => void }) {
  return (
    <Field
      label="Approval cap"
      htmlFor="approval-cap"
      hint="How many requests the chief may approve before the rest ask you. Up to 1000."
    >
      <input
        id="approval-cap"
        type="number"
        min={0}
        max={1000}
        className={cx(inputClass, "w-28 tabular-nums")}
        value={value}
        onChange={(event) => onChange(event.target.value)}
      />
    </Field>
  );
}

function parseCap(text: string): number | null {
  const cap = Number(text);
  return text.trim() !== "" && Number.isInteger(cap) && cap >= 0 && cap <= 1000 ? cap : null;
}

function NewRunSheet({ workspaceId }: { workspaceId: number }) {
  const { cos, ui } = useApp();
  const workspace = useCos((s) => workspaceOf(s, workspaceId));
  const { busy, error, run, close } = useAction();
  const [goal, setGoal] = useState("");
  const [preset, setPreset] = useState("normal");
  const [cap, setCap] = useState("20");
  if (!workspace) return null;
  const capValue = parseCap(cap);
  const ok = goal.trim() !== "" && capValue !== null;
  return (
    <Dialog
      width="lg"
      title="New run"
      description={`A chief plans the goal in ${workspaceName(workspace)} as tasks. You approve the plan before any worker starts.`}
      onClose={close}
      onSubmit={() =>
        ok &&
        run(async () => {
          const created = await cos.createRun({
            workspaceId,
            goal: goal.trim(),
            preset,
            approvalCap: capValue ?? undefined,
          });
          ui.navigate({ kind: "run", id: created.id });
        })
      }
      footer={<Footer busy={busy} label="Start run" disabled={!ok} onCancel={close} />}
    >
      <Field label="Goal" htmlFor="run-goal">
        <textarea
          id="run-goal"
          rows={3}
          className={cx(inputClass, "resize-none leading-relaxed")}
          placeholder="Make the dashboard load in under a second"
          value={goal}
          onChange={(event) => setGoal(event.target.value)}
        />
      </Field>
      <PresetPicker value={preset} onChange={setPreset} />
      <CapField value={cap} onChange={setCap} />
      <ErrorText>{error}</ErrorText>
    </Dialog>
  );
}

function CancelRun({ runId }: { runId: number }) {
  const { cos } = useApp();
  const target = useCos((s) => runOf(s, runId));
  const { busy, error, run, close } = useAction();
  const [reason, setReason] = useState("");
  if (!target) return null;
  return (
    <Dialog
      title="Cancel this run?"
      description="Its agents stop and unfinished tasks are cancelled. Merged work stays on the workspace branch."
      onClose={close}
      onSubmit={() => run(() => cos.cancelRun(runId, reason.trim() || undefined))}
      footer={<Footer busy={busy} label="Cancel run" danger onCancel={close} />}
    >
      <Field label="Reason" htmlFor="cancel-reason" hint="Optional. Shown on the run.">
        <input
          id="cancel-reason"
          className={inputClass}
          value={reason}
          onChange={(event) => setReason(event.target.value)}
        />
      </Field>
      <ErrorText>{error}</ErrorText>
    </Dialog>
  );
}

const taskCopy: Record<TaskActionKind, { title: string; button: string; description: string }> = {
  retry: {
    title: "Retry this task?",
    button: "Retry",
    description: "A fresh worker starts on it, with your reason. Each task gets one retry.",
  },
  fail: {
    title: "Mark this task failed?",
    button: "Mark failed",
    description: "Tasks that depend on it can't start. The chief hears about it.",
  },
  cancel: {
    title: "Cancel this task?",
    button: "Cancel task",
    description: "Its worker stops and its worktree is dropped.",
  },
};

function TaskAction({
  runId,
  taskId,
  action,
}: {
  runId: number;
  taskId: number;
  action: TaskActionKind;
}) {
  const { cos } = useApp();
  const task = useCos((s) => s.boards[runId]?.tasks.find((t) => t.id === taskId));
  const { busy, error, run, close } = useAction();
  const [reason, setReason] = useState("");
  if (!task) return null;
  const copy = taskCopy[action];
  const required = needsReason(action);
  const ok = !required || reason.trim() !== "";
  return (
    <Dialog
      title={copy.title}
      description={
        <>
          <span className="font-medium text-ink">{task.title}</span>. {copy.description}
        </>
      }
      onClose={close}
      onSubmit={() =>
        ok && run(() => cos.updateTask(task, targetState(action), reason.trim() || undefined))
      }
      footer={
        <Footer
          busy={busy}
          label={copy.button}
          danger={action !== "retry"}
          disabled={!ok}
          onCancel={close}
        />
      }
    >
      <Field
        label={required ? "Reason" : "Reason (optional)"}
        htmlFor="task-reason"
        hint={action === "retry" ? "The new worker reads it." : undefined}
      >
        <textarea
          id="task-reason"
          rows={2}
          className={cx(inputClass, "resize-none")}
          value={reason}
          onChange={(event) => setReason(event.target.value)}
        />
      </Field>
      {task.lastFailure ? (
        <p className="rounded-lg bg-subtle px-3 py-2 font-mono text-[12px] text-ink-2 select-text">
          {task.lastFailure}
        </p>
      ) : null}
      <ErrorText>{error}</ErrorText>
    </Dialog>
  );
}

function LimitsSheet({ runId }: { runId: number }) {
  const { cos } = useApp();
  const target = useCos((s) => runOf(s, runId));
  const limits = useCos((s) => s.limits[runId]);
  const { busy, error, run, close } = useAction();
  const [preset, setPreset] = useState(limits?.preset ?? target?.preset ?? "normal");
  const [cap, setCap] = useState(String(limits?.approvalCap ?? target?.approvalCap ?? 20));
  if (!target) return null;
  const capValue = parseCap(cap);
  return (
    <Dialog
      title="Run limits"
      description={
        limits
          ? `The chief has approved ${limits.chiefApprovals} so far. New limits apply from its next answer.`
          : "New limits apply from the chief's next answer."
      }
      onClose={close}
      onSubmit={() => capValue !== null && run(() => cos.setLimits(runId, preset, capValue))}
      footer={<Footer busy={busy} label="Save" disabled={capValue === null} onCancel={close} />}
    >
      <PresetPicker value={preset} onChange={setPreset} />
      <CapField value={cap} onChange={setCap} />
      <ErrorText>{error}</ErrorText>
    </Dialog>
  );
}
