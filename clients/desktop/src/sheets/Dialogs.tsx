// Every sheet the app opens, chosen by the UI store's `dialog`. Each one
// shows the daemon's error as-is and stays open until the action succeeds.

import { useState } from "react";
import { isConflict, messageOf } from "../api/errors";
import { projectSlug, workspaceName } from "../api/models";
import { useApp, useCanChange, useCos, useUi } from "../app/context";
import type { DialogState, Page } from "../app/ui";
import { Composer } from "../screens/Composer";
import { toNewAgent } from "../screens/composerValue";
import { isAlive, pluralize } from "../domain/labels";
import {
  agent as agentOf,
  isUnder,
  project as projectOf,
  workspace as workspaceOf,
  workspacesNewestFirst,
} from "../store/selectors";
import { cx } from "../ui/cx";
import { Dialog, ErrorText } from "../ui/Dialog";
import { Button, Field, inputClass } from "../ui/kit";
import { EnvVarSheet } from "./EnvSheet";
import { AddHostSheet, RemoveHostSheet, RenameHostSheet } from "./HostSheets";

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
    case "newSession":
      return <NewSessionSheet projectId={dialog.projectId} />;
    case "removeAgent":
      return <RemoveAgent agentId={dialog.agentId} />;
    case "envVar":
      return <EnvVarSheet dialog={dialog} />;
    case "addHost":
      return <AddHostSheet />;
    case "renameHost":
      return <RenameHostSheet hostId={dialog.hostId} />;
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
          for (const kind of ["chief", "project", "projectEnv", "chiefMemory"] as const)
            ui.closeTab({ kind, id: projectId });
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
          ui.closeTab({ kind: "workspace", id: workspace.id });
        })
      }
      footer={<Footer busy={busy} label="Remove" danger onCancel={close} />}
    >
      <ErrorText>{error}</ErrorText>
    </Dialog>
  );
}

// MARK: Agents

/**
 * A new session from the sidebar's +: Claude Code in a folder of its own (a
 * new workspace), or in one another session already works in.
 */
function NewSessionSheet({ projectId }: { projectId: number }) {
  const { cos, ui } = useApp();
  const project = useCos((s) => projectOf(s, projectId));
  const agents = useCos((s) => s.agents);
  const canChange = useCanChange();
  const [folder, setFolder] = useState<"new" | number>("new");
  const [error, setError] = useState<string | null>(null);
  if (!project) return null;
  const base = project.hasOrigin ? "origin" : "head";
  const workspaces = workspacesNewestFirst(project);
  const count = (id: number) => agents.filter((a) => a.workspaceId === id).length;
  return (
    <Dialog
      width="lg"
      title={`New session in ${project.name}`}
      description="Claude Code starts in a folder of its own, on its own branch, unless you pick one another session works in."
      onClose={ui.close}
    >
      <Field label="Folder" htmlFor="session-folder">
        <select
          id="session-folder"
          className={inputClass}
          value={String(folder)}
          onChange={(event) =>
            setFolder(event.target.value === "new" ? "new" : Number(event.target.value))
          }
        >
          <option value="new">
            A new folder, from {base === "origin" ? `origin/${project.baseBranch}` : "your HEAD"}
          </option>
          {workspaces.map((w) => (
            <option key={w.id} value={w.id}>
              {workspaceName(w)}
              {count(w.id) > 0 ? ` · shared with ${pluralize(count(w.id), "session")}` : ""}
            </option>
          ))}
        </select>
      </Field>
      <Composer
        autoFocus
        attachTo={folder === "new" ? null : { kind: "workspace", workspaceId: folder }}
        disabled={!canChange}
        onSend={async (value) => {
          try {
            const workspaceId =
              folder === "new" ? (await cos.createWorkspace(projectId, base)).id : folder;
            const agent = await cos.createAgent(workspaceId, toNewAgent(value));
            ui.close();
            ui.navigate({ kind: "agent", id: agent.id });
          } catch (failure: unknown) {
            setError(messageOf(failure));
            throw failure;
          }
        }}
      />
      <ErrorText>{error}</ErrorText>
    </Dialog>
  );
}

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
      title={`New session in ${workspaceName(workspace)}`}
      description="Claude Code starts in this folder, beside the sessions already there. Leave the prompt empty to start it idle."
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
        attachTo={{ kind: "workspace", workspaceId }}
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

/**
 * Removes a session. A running one is stopped first. When no other session
 * works in its folder, the folder (the worktree) goes too, unless you untick
 * it; the daemon keeps a folder with uncommitted changes, and so does this.
 */
function RemoveAgent({ agentId }: { agentId: number }) {
  const { cos, ui } = useApp();
  const agent = useCos((s) => agentOf(s, agentId));
  const workspace = useCos((s) => workspaceOf(s, agent?.workspaceId));
  const agents = useCos((s) => s.agents);
  const [withFolder, setWithFolder] = useState(true);
  const { busy, error, run, close } = useAction();
  if (!agent) return null;
  const alive = isAlive(agent.state);
  const others = workspace
    ? agents.filter((a) => a.workspaceId === workspace.id && a.id !== agent.id)
    : [];
  const name = workspace ? workspaceName(workspace) : null;
  const deletesFolder = workspace !== undefined && others.length === 0 && withFolder;

  const remove = async () => {
    if (alive) {
      await cos.stopAgent(agent.id);
      await cos.untilStopped(agent.id);
    }
    if (deletesFolder) {
      await cos.removeWorkspace(workspace).catch((failure: unknown) => {
        if (!isConflict(failure)) throw failure;
        throw new Error(
          `${messageOf(failure)}. The folder stays. Untick “Delete its folder” to remove just the session.`,
        );
      });
    } else {
      await cos.removeAgent(agent.id);
    }
    // Its tabs go: the session's, and with the folder, the folder's and its terminals'.
    const gone = (page: Page) =>
      (page.kind === "agent" && page.id === agent.id) ||
      (deletesFolder &&
        ((page.kind === "workspace" && page.id === workspace.id) ||
          (page.kind === "shell" && isUnder(page.cwd, workspace.path))));
    for (const tab of ui.store.getState().tabs.filter(gone)) ui.closeTab(tab);
  };

  return (
    <Dialog
      width="sm"
      title={`Remove ${agent.title}?`}
      description={
        <>
          {alive ? "It's still running, so it stops first. " : null}
          cos forgets the session and its terminal. Claude's own session history stays.
          {name && others.length > 0 ? (
            <>
              {" "}
              {name} stays: {others.map((a) => a.title).join(", ")}{" "}
              {others.length === 1 ? "works" : "work"} there too.
            </>
          ) : null}
        </>
      }
      onClose={close}
      onSubmit={() => run(remove)}
      footer={
        <Footer busy={busy} label={alive ? "Stop and remove" : "Remove"} danger onCancel={close} />
      }
    >
      {workspace && others.length === 0 ? (
        <label className="flex items-start gap-2.5 text-[13px] text-ink">
          <input
            type="checkbox"
            className="mt-0.5 size-4 accent-[var(--c-accent)]"
            checked={withFolder}
            onChange={(event) => setWithFolder(event.target.checked)}
          />
          <span className="flex flex-col gap-0.5">
            <span>Delete its folder, {name}, too</span>
            <span className="text-[12px] leading-snug text-ink-3">
              <code className="font-mono text-[11.5px]">{workspace.path}</code>, unless it has
              uncommitted changes. The branch{" "}
              <code className="font-mono text-[11.5px]">{workspace.branch}</code> is kept, so no
              commit is lost.
            </span>
          </span>
        </label>
      ) : null}
      <ErrorText>{error}</ErrorText>
    </Dialog>
  );
}
