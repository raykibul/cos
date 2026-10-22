// A project's Chief of Staff, as a chat (docs/chief-of-staff.md, "App"):
// a header with its state, how full its context is, model, terminal toggle
// and menu; the conversation from its transcript; the agents waiting on you;
// whether coordination is paused; and a composer. Its own terminal opens in
// a resizable split on the right (Ctrl-J).

import {
  ChevronDown,
  FolderGit2,
  LayoutGrid,
  MoreHorizontal,
  Pause,
  Play,
  RotateCcw,
  Sparkles,
  Square,
  SquareTerminal,
  UserRound,
  Variable,
  X,
} from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { CLAUDE_CONFIG_DIR, envHint } from "../../api/env";
import { useApp, useCanChange, useCos, useUi } from "../../app/context";
import { models, pluralize } from "../../domain/labels";
import { type PendingMessage, conversationItems } from "../../domain/transcript";
import {
  chiefRequests,
  claudeAccount,
  projectEnvNames,
  project as projectOf,
  spawnedAgents,
  staffAgent,
  staffState,
} from "../../store/selectors";
import { AgentTerminal } from "../../terminal/AgentTerminal";
import { folderItems } from "../../screens/common";
import { ChiefMark } from "../../screens/sidebar/ProjectSection";
import { cx } from "../../ui/cx";
import { EmptyState, IconButton, Spinner, StateDot } from "../../ui/kit";
import { type MenuItem, Menu } from "../../ui/Menu";
import { PageHeader } from "../../ui/Page";
import { ChiefComposer } from "./ChiefComposer";
import { ContextRing } from "./ContextRing";
import { Conversation } from "./Conversation";
import { PausedStrip } from "./PausedStrip";
import { SplitDivider } from "./SplitDivider";
import { staffLabel, staffRunning, suggestions } from "./staffState";
import { WaitingStrip } from "./WaitingStrip";

const noPending: PendingMessage[] = [];

export function ChiefPage({ projectId }: { projectId: number }) {
  const project = useCos((s) => projectOf(s, projectId));
  const loaded = useCos((s) => s.projectsLoaded);
  const supported = useCos((s) => s.staffSupported);
  if (!project || !supported) {
    return (
      <>
        <PageHeader />
        <EmptyState
          icon={FolderGit2}
          className="flex-1"
          title={
            !supported
              ? "This daemon has no Chief of Staff"
              : loaded
                ? "This project is gone"
                : "Loading…"
          }
        >
          {!supported
            ? "Update cos to talk to a Chief of Staff for each project."
            : loaded
              ? "It was removed. Pick another one in the sidebar."
              : null}
        </EmptyState>
      </>
    );
  }
  return <ChiefView projectId={projectId} projectName={project.name} />;
}

function ChiefView({ projectId, projectName }: { projectId: number; projectName: string }) {
  const { cos, ui } = useApp();
  const canChange = useCanChange();
  const staff = useCos((s) => s.staffs[projectId]);
  const agent = useCos((s) => staffAgent(s, projectId));
  const state = useCos((s) => staffState(s, projectId));
  const agentId = staff?.agentId ?? null;
  const transcript = useCos((s) => (agentId !== null ? s.transcripts[agentId] : undefined));
  const pending = useCos((s) => s.pending[projectId] ?? noPending);
  const inbox = useCos((s) => s.inboxes[projectId]);
  const view = useCos((s) => ({ agents: s.agents, approvals: s.approvals, staffs: s.staffs }));
  const terminalOpen = useUi((s) => s.chiefTerminal);
  const envNames = useCos((s) => projectEnvNames(s, projectId));
  const account = useCos((s) => claudeAccount(s, projectId));
  const env = envHint(envNames);
  const [draft, setDraft] = useState("");
  const input = useRef<HTMLTextAreaElement>(null);
  const [jumps, setJumps] = useState(0);

  useEffect(() => (agentId !== null ? cos.watchTranscript(agentId) : undefined), [cos, agentId]);
  useEffect(() => cos.watchInbox(projectId), [cos, projectId]);

  // Ctrl-J shows and hides the chief's terminal, as in Selene.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.ctrlKey && !event.metaKey && !event.altKey && event.key.toLowerCase() === "j") {
        event.preventDefault();
        ui.toggleChiefTerminal();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [ui]);

  const items = useMemo(
    () => conversationItems(transcript?.messages ?? [], pending),
    [transcript?.messages, pending],
  );
  const requests = useMemo(() => chiefRequests(view, projectId), [view, projectId]);
  const spawned = useMemo(() => spawnedAgents(view, agentId), [view, agentId]);
  const working = spawned.filter((a) => a.state === "working" || a.state === "starting").length;
  const sending = pending.some((p) => p.state === "sending");
  const label = staffLabel(state);
  const running = staffRunning(state);
  const loading = agentId !== null && !transcript?.loaded && items.length === 0;

  const send = () => {
    const text = draft.trim();
    if (!text) return;
    setDraft("");
    setJumps((n) => n + 1);
    void cos.messageStaff(projectId, text);
  };

  const suggest = (text: string) => {
    setDraft(text);
    requestAnimationFrame(() => {
      const el = input.current;
      if (!el) return;
      el.focus();
      el.setSelectionRange(text.length, text.length);
    });
  };

  const model = models.find((m) => m.value === (staff?.model ?? ""));
  const modelTitle = model?.short ?? staff?.model ?? "Default";

  const menu: MenuItem[] = [
    running
      ? {
          label: "Restart",
          icon: RotateCcw,
          detail: "Stops it and resumes the same conversation",
          disabled: !canChange,
          onSelect: () =>
            void ui.perform("Couldn't restart the Chief of Staff", () =>
              cos.restartStaff(projectId),
            ),
        }
      : {
          label: agentId === null ? "Start" : "Resume",
          icon: Play,
          disabled: !canChange,
          onSelect: () =>
            void ui.perform("Couldn't start the Chief of Staff", () => cos.startStaff(projectId)),
        },
    {
      label: "Stop",
      icon: Square,
      disabled: !canChange || !running || state === "starting",
      onSelect: () =>
        void ui.perform("Couldn't stop the Chief of Staff", () =>
          agentId !== null ? cos.stopAgent(agentId) : Promise.resolve(),
        ),
    },
    staff?.paused
      ? {
          label: "Resume coordination",
          icon: Play,
          detail: "Updates from its agents go in again",
          disabled: !canChange,
          onSelect: () =>
            void ui.perform("Couldn't resume coordination", () =>
              cos.setStaffPaused(projectId, false),
            ),
        }
      : {
          label: "Pause coordination",
          icon: Pause,
          detail: "Updates wait until you resume; your messages still go through",
          disabled: !canChange || !staff,
          onSelect: () =>
            void ui.perform("Couldn't pause coordination", () =>
              cos.setStaffPaused(projectId, true),
            ),
        },
    "separator",
    {
      label: "All workspaces",
      icon: LayoutGrid,
      onSelect: () => ui.navigate({ kind: "project", id: projectId }),
    },
    {
      label: "Environment…",
      icon: Variable,
      detail: "Variables for this project's agents",
      onSelect: () => ui.navigate({ kind: "projectEnv", id: projectId }),
    },
    ...(staff?.worktree ? folderItems(cos, staff.worktree) : []),
  ];

  return (
    <div className="flex min-h-0 flex-1">
      <section className="@container flex min-w-0 flex-1 flex-col" aria-label="Chief of Staff">
        <header
          data-tauri-drag-region
          className="flex h-[52px] shrink-0 items-center gap-3 border-b border-line px-5 select-none"
        >
          <ChiefMark size="md" />
          <div data-tauri-drag-region className="min-w-0 flex-1">
            <div className="flex min-w-0 items-center gap-2">
              <h1 className="truncate text-[13.5px] leading-tight font-semibold text-ink">
                Chief of Staff
                <span className="font-normal text-ink-3"> · {projectName}</span>
              </h1>
              {account ? (
                <span
                  aria-label={`Claude account: ${account.account}`}
                  title={`The Claude account this project's agents use, from ${CLAUDE_CONFIG_DIR}${account.inherited ? " for all projects" : ""}`}
                  className="hidden h-[18px] min-w-0 shrink items-center gap-1 rounded-full bg-subtle px-1.5 font-mono text-[10.5px] text-ink-3 @min-[34rem]:inline-flex"
                >
                  <UserRound className="size-2.5 shrink-0" strokeWidth={2} aria-hidden />
                  <span className="truncate">{account.account}</span>
                </span>
              ) : null}
            </div>
            <div className="mt-0.5 flex min-w-0 items-center gap-1.5 text-[12px] leading-tight text-ink-3">
              <p className="flex min-w-0 items-center gap-1.5" role="status">
                <StateDot label={label} className="size-1.5 shrink-0" />
                <span className="truncate">
                  <span>{label.title}</span>
                  {working > 0 ? <span> · {pluralize(working, "agent")} working</span> : null}
                </span>
              </p>
              {env ? (
                <button
                  type="button"
                  onClick={() => ui.navigate({ kind: "projectEnv", id: projectId })}
                  title={`Environment for ${projectName}'s agents:\n${envNames.join("\n")}`}
                  className="min-w-0 truncate rounded px-0.5 hover:bg-subtle hover:text-ink-2"
                >
                  · {env}
                </button>
              ) : null}
            </div>
          </div>
          <div className="flex shrink-0 items-center gap-1">
            <ContextRing usage={transcript?.context} />
            <Menu
              align="end"
              width={220}
              items={models.map((m) => ({
                label: m.title,
                checked: m.value === (staff?.model ?? ""),
                onSelect: () =>
                  void ui.perform("Couldn't change the model", () =>
                    cos.setStaffModel(projectId, m.value || null),
                  ),
              }))}
              trigger={(props) => (
                <button
                  type="button"
                  {...props}
                  disabled={!canChange || !staff}
                  aria-label={`Model: ${modelTitle}`}
                  title="The chief's model. A change applies from its next start."
                  className="flex h-7 items-center gap-1.5 rounded-lg border border-line-strong px-2.5 text-[12px] text-ink-2 hover:bg-subtle hover:text-ink disabled:opacity-45 aria-expanded:bg-subtle"
                >
                  <Sparkles className="size-3.5" strokeWidth={1.8} />
                  {modelTitle}
                  <ChevronDown className="size-3 text-ink-3" strokeWidth={2} />
                </button>
              )}
            />
            <IconButton
              icon={SquareTerminal}
              label={terminalOpen ? "Hide the chief's terminal" : "Show the chief's terminal"}
              title={`${terminalOpen ? "Hide" : "Show"} the chief's terminal (Ctrl-J)`}
              aria-pressed={terminalOpen}
              disabled={agentId === null}
              onClick={() => ui.toggleChiefTerminal()}
              className={cx("ml-1 size-8", terminalOpen && "bg-subtle-2 text-ink")}
            />
            <Menu
              align="end"
              width={240}
              items={menu}
              trigger={(props) => (
                <button
                  type="button"
                  aria-label="Chief of Staff actions"
                  {...props}
                  className="flex size-8 items-center justify-center rounded-lg text-ink-2 hover:bg-subtle hover:text-ink"
                >
                  <MoreHorizontal className="size-4" strokeWidth={1.8} />
                </button>
              )}
            />
          </div>
        </header>

        {loading ? (
          <div className="flex flex-1 items-center justify-center">
            <Spinner />
          </div>
        ) : (
          <Conversation
            items={items}
            inbox={inbox ?? []}
            working={state === "working" || (state === "starting" && pending.length > 0)}
            jumpSignal={jumps}
            onRetry={(p) => void cos.retryPending(projectId, p.id)}
            onDiscard={(p) => cos.discardPending(projectId, p.id)}
            empty={
              <Welcome
                projectName={projectName}
                error={transcript?.error ?? null}
                onSuggest={suggest}
              />
            }
          />
        )}

        <div className="mx-auto w-full max-w-[740px] shrink-0 px-8 pb-5">
          <WaitingStrip requests={requests} chiefAgentId={agentId} />
          {staff?.paused ? (
            <PausedStrip projectId={projectId} waiting={staff.updatesWaiting} />
          ) : null}
          <ChiefComposer
            ref={input}
            value={draft}
            onChange={setDraft}
            onSend={send}
            sending={sending}
            disabled={!canChange}
            note={
              state === "working" && draft.trim()
                ? "It's working: your message waits until it's free"
                : null
            }
          />
        </div>
      </section>

      {terminalOpen && agentId !== null ? <SplitDivider /> : null}
      {terminalOpen && agentId !== null ? (
        <aside
          aria-label="The chief's terminal"
          className="flex w-[46%] max-w-full min-w-0 shrink-0 flex-col bg-page"
        >
          <div
            data-tauri-drag-region
            className="flex h-[52px] shrink-0 items-center gap-2 border-b border-line px-4 text-[12.5px] select-none"
          >
            <SquareTerminal className="size-4 text-ink-3" strokeWidth={1.8} aria-hidden />
            <span className="font-medium text-ink">Claude's terminal</span>
            <span className="truncate text-ink-3">
              · {agent?.model ?? staff?.model ?? "default model"}
            </span>
            <button
              type="button"
              onClick={() => ui.navigate({ kind: "agent", id: agentId })}
              className="ml-auto rounded-md px-2 py-1 text-ink-3 hover:bg-subtle hover:text-ink"
            >
              Open full page
            </button>
            <IconButton
              icon={X}
              label="Hide the chief's terminal"
              onClick={() => ui.toggleChiefTerminal(false)}
            />
          </div>
          <div className="flex min-h-0 flex-1 flex-col p-3">
            <AgentTerminal
              key={agentId}
              agentId={agentId}
              alive={running}
              autoFocus={false}
              className="flex-1"
            />
          </div>
        </aside>
      ) : null}
    </div>
  );
}

function Welcome({
  projectName,
  error,
  onSuggest,
}: {
  projectName: string;
  error: string | null;
  onSuggest: (text: string) => void;
}) {
  return (
    <div className="flex flex-col items-start gap-3 pt-[9vh] pb-6">
      <ChiefMark size="lg" />
      <h2 className="mt-1.5 font-serif text-[24px] leading-tight font-medium tracking-[-0.01em] text-ink">
        Coordinate {projectName}
      </h2>
      <p className="max-w-[560px] text-[14px] leading-relaxed text-ink-2">
        The Chief of Staff plans the work, starts agents in their own workspaces, and brings their
        questions and results back here. It starts with your first message.
      </p>
      {error ? <p className="text-[13px] text-tone-red">{error}</p> : null}
      <div className="mt-3 flex w-full flex-col gap-1.5">
        {suggestions.map((text) => (
          <button
            key={text}
            type="button"
            onClick={() => onSuggest(text)}
            className="rounded-xl border border-line-strong px-3.5 py-2.5 text-left text-[13.5px] text-ink transition-colors hover:border-ink-3/40 hover:bg-subtle"
          >
            {text}
          </button>
        ))}
      </div>
    </div>
  );
}
