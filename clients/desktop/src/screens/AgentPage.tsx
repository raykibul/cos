import {
  ArrowUpRight,
  Bot,
  KeyRound,
  MessageSquare,
  MoreHorizontal,
  Play,
  ShieldQuestion,
  Square,
  SquareTerminal,
  Trash2,
  X,
} from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { envHint, sortedNames } from "../api/env";
import { type Agent, type Approval, workspaceName } from "../api/models";
import type { InboxItem } from "../api/staff";
import { useApp, useCanChange, useCos, useUi } from "../app/context";
import { useFocusOnOpen } from "../app/focus";
import { type PendingMessage, conversationItems } from "../domain/transcript";
import { Conversation, type Speaker } from "../pages/chief/Conversation";
import { SplitDivider } from "../pages/chief/SplitDivider";
import { ChatComposer } from "../ui/ChatComposer";
import { useDraftFiles } from "../ui/draftFiles";
import { approvalDetail, approvalHeadline, routeExplanation } from "../domain/approvals";
import { loginCommand } from "../domain/env";
import { agentLabel, canResume, isAlive, permissionModeTitle } from "../domain/labels";
import {
  agent as agentOf,
  configDirOf,
  pendingApproval,
  project as projectOf,
  projectOfPage,
  workspace as workspaceOf,
} from "../store/selectors";
import { AgentTerminal } from "../terminal/AgentTerminal";
import { cx } from "../ui/cx";
import { Button, EmptyState, IconButton, PathText, Spinner, StateBadge } from "../ui/kit";
import { type MenuItem, Menu } from "../ui/Menu";
import { Crumb, CrumbSep, PageHeader } from "../ui/Page";
import { folderItems } from "./common";
import { ApprovalButtons } from "./shared";

export function AgentPage({ agentId }: { agentId: number }) {
  const agent = useCos((s) => agentOf(s, agentId));
  const loaded = useCos((s) => s.projectsLoaded && s.agents.length > 0);
  if (!agent) {
    return (
      <>
        <PageHeader />
        <EmptyState
          icon={Bot}
          title={loaded ? "This agent is gone" : "Loading…"}
          className="flex-1"
        >
          {loaded ? "It was removed. Pick another one in the sidebar." : null}
        </EmptyState>
      </>
    );
  }
  return <AgentView agent={agent} />;
}

/**
 * A session shown beside the chief's chat, opened from its card: the same
 * conversation and composer as its page, under a compact header with Open
 * in tab and close. Its terminal takes the panel's place instead of a
 * second split.
 */
export function AgentPanel({ agentId, onClose }: { agentId: number; onClose: () => void }) {
  const agent = useCos((s) => agentOf(s, agentId));
  if (!agent) {
    return (
      <div className="flex min-h-0 flex-1 flex-col">
        <div className="flex h-[52px] shrink-0 items-center justify-end px-3">
          <IconButton icon={X} label="Close the session" onClick={onClose} />
        </div>
        <EmptyState icon={Bot} title="This session is gone" className="flex-1">
          It was removed.
        </EmptyState>
      </div>
    );
  }
  return <AgentView key={agent.id} agent={agent} embedded={{ onClose }} />;
}

function AgentView({
  agent,
  embedded,
}: {
  agent: Agent;
  /** Shown beside the chief's chat rather than as a page. */
  embedded?: { onClose: () => void };
}) {
  const { cos, ui } = useApp();
  const canChange = useCanChange();
  const workspace = useCos((s) => workspaceOf(s, agent.workspaceId));
  // A Chief of Staff has no workspace; its project is the one it serves.
  const project = useCos((s) => {
    const id = workspace?.projectId ?? projectOfPage(s, { kind: "agent", id: agent.id });
    return id !== null ? projectOf(s, id) : undefined;
  });
  const approval = useCos((s) => pendingApproval(s, agent.id));
  const transcript = useCos((s) => s.transcripts[agent.id]);
  const pending = useCos((s) => s.agentPending[agent.id] ?? noPending);
  const pageTerminal = useUi((s) => s.agentTerminal);
  const panelTerminal = useUi((s) => s.sessionTerminal);
  const terminalWanted = embedded ? panelTerminal : pageTerminal;
  const toggleTerminal = (open: boolean) =>
    embedded ? ui.showSessionTerminal(open) : ui.toggleAgentTerminal(open);
  const [busy, setBusy] = useState<"stop" | "resume" | null>(null);
  const [draft, setDraft] = useState("");
  const [jumps, setJumps] = useState(0);
  const target = useMemo(() => ({ kind: "agent" as const, agentId: agent.id }), [agent.id]);
  const files = useDraftFiles(canChange ? target : null);
  const input = useRef<HTMLTextAreaElement>(null);
  const alive = isAlive(agent.state);
  const label = agentLabel(agent.state);
  const cwd = agent.cwd ?? workspace?.path ?? null;

  // The chief's own conversation is on its page, so its agent page is the
  // terminal; so is any agent on a daemon that can't read transcripts.
  const noTranscripts = Boolean(transcript?.error) && (transcript?.messages.length ?? 0) === 0;
  const terminalOnly = agent.role === "staff" || noTranscripts;
  // Logging in happens in the terminal, so it shows while Claude asks.
  const terminalOpen = terminalOnly || terminalWanted || agent.state === "needs_login";
  useFocusOnOpen(input, !terminalOnly && !embedded);
  // Beside the chief, its message box takes the cursor when you open it
  // (after the chief's page has put it in its own).
  const focusWanted = useUi((s) => embedded !== undefined && s.focusSession === agent.id);
  useEffect(() => {
    if (!focusWanted) return;
    const frame = requestAnimationFrame(() => {
      if (!ui.store.getState().dialog) input.current?.focus({ preventScroll: true });
      ui.sessionFocused();
    });
    return () => cancelAnimationFrame(frame);
  }, [ui, focusWanted]);

  useEffect(() => cos.watchTranscript(agent.id), [cos, agent.id]);

  // Ctrl-Shift-J shows and hides the terminal, as on the chief's page.
  useEffect(() => {
    if (terminalOnly || embedded) return;
    const onKey = (event: KeyboardEvent) => {
      // Plain Ctrl-J opens a shell tab (App.tsx).
      if (
        event.ctrlKey &&
        event.shiftKey &&
        !event.metaKey &&
        !event.altKey &&
        event.code === "KeyJ"
      ) {
        event.preventDefault();
        ui.toggleAgentTerminal();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [ui, terminalOnly, embedded]);

  const items = useMemo(
    () => conversationItems(transcript?.messages ?? [], pending),
    [transcript?.messages, pending],
  );
  const loading = !transcript?.loaded && items.length === 0;
  const sending = pending.some((p) => p.state === "sending");
  const blocked = composerBlock(agent.state);

  const act = (kind: "stop" | "resume") => {
    setBusy(kind);
    void ui
      .perform(kind === "stop" ? "Couldn't stop the agent" : "Couldn't resume the agent", () =>
        kind === "stop" ? cos.stopAgent(agent.id) : cos.resumeAgent(agent.id),
      )
      .finally(() => setBusy(null));
  };

  const send = () => {
    const text = draft.trim();
    const attached = files.ready;
    if ((!text && attached.length === 0) || files.uploading) return;
    setDraft("");
    files.clear();
    setJumps((n) => n + 1);
    void cos.messageAgent(agent.id, text, attached);
  };

  const menu: MenuItem[] = [
    ...(cwd ? folderItems(cos, cwd) : []),
    "separator",
    {
      label: "Remove session…",
      icon: Trash2,
      danger: true,
      disabled: !canChange,
      detail: alive ? "Stops it first" : undefined,
      onSelect: () => ui.open({ kind: "removeAgent", agentId: agent.id }),
    },
  ];

  const header = embedded ? (
    <div
      data-tauri-drag-region
      className="flex h-[52px] shrink-0 items-center gap-2 border-b border-line px-3 text-[12.5px] select-none"
    >
      <Bot className="size-4 shrink-0 text-tone-blue" strokeWidth={1.9} aria-hidden />
      <h2 className="min-w-0 truncate text-[13px] font-semibold text-ink">{agent.title}</h2>
      <StateBadge label={label} className="shrink-0" />
      <span className="ml-auto flex shrink-0 items-center gap-0.5">
        {terminalOnly ? null : <ViewSwitch terminal={terminalOpen} onChange={toggleTerminal} />}
        {canResume(agent.state) ? (
          <IconButton
            icon={Play}
            label="Resume"
            disabled={!canChange || !agent.sessionId || busy !== null}
            onClick={() => act("resume")}
          />
        ) : null}
        <IconButton
          icon={ArrowUpRight}
          label="Open in a tab"
          onClick={() => ui.navigate({ kind: "agent", id: agent.id })}
        />
        <IconButton icon={X} label="Close the session" onClick={embedded.onClose} />
      </span>
    </div>
  ) : (
    <PageHeader
      actions={
        <>
          {terminalOnly ? null : (
            <IconButton
              icon={SquareTerminal}
              label={terminalOpen ? "Hide the terminal" : "Show the terminal"}
              title={`${terminalOpen ? "Hide" : "Show"} Claude's terminal (Ctrl-Shift-J)`}
              aria-pressed={terminalOpen}
              onClick={() => toggleTerminal(!terminalOpen)}
              className={cx("size-8", terminalOpen && "bg-subtle-2 text-ink")}
            />
          )}
          {alive ? (
            <Button
              icon={Square}
              busy={busy === "stop"}
              disabled={!canChange || agent.state === "starting"}
              onClick={() => act("stop")}
              title="Sends /exit to Claude"
            >
              Stop
            </Button>
          ) : null}
          {!alive ? (
            <Button
              variant="ghost"
              icon={Trash2}
              disabled={!canChange}
              onClick={() => ui.open({ kind: "removeAgent", agentId: agent.id })}
            >
              Remove
            </Button>
          ) : null}
          {canResume(agent.state) ? (
            <Button
              variant="primary"
              icon={Play}
              busy={busy === "resume"}
              disabled={!canChange || !agent.sessionId}
              title={
                agent.sessionId
                  ? "Starts Claude again on this conversation"
                  : "No session to resume"
              }
              onClick={() => act("resume")}
            >
              Resume
            </Button>
          ) : null}
          <Menu
            align="end"
            items={menu}
            trigger={(props) => (
              <button
                type="button"
                aria-label="Agent actions"
                {...props}
                className="flex size-8 items-center justify-center rounded-lg text-ink-2 hover:bg-subtle hover:text-ink"
              >
                <MoreHorizontal className="size-4" strokeWidth={1.8} />
              </button>
            )}
          />
        </>
      }
    >
      {project ? (
        <>
          <Crumb onClick={() => ui.navigate({ kind: "chief", id: project.id })}>
            {project.name}
          </Crumb>
          <CrumbSep />
        </>
      ) : null}
      {workspace ? (
        <span className="hidden min-w-0 items-center gap-1.5 @min-[40rem]:flex">
          <Crumb onClick={() => ui.navigate({ kind: "workspace", id: workspace.id })}>
            {workspaceName(workspace)}
          </Crumb>
          <CrumbSep />
        </span>
      ) : null}
      <span className="truncate font-medium text-ink">{agent.title}</span>
      <StateBadge label={label} className="ml-1.5" />
      <span className="ml-1 hidden shrink-0 text-[12px] text-ink-3 @min-[48rem]:inline">
        {agent.model ?? "default model"}
      </span>
    </PageHeader>
  );

  if (terminalOnly || (embedded && terminalOpen)) {
    return (
      <div className="@container flex min-h-0 flex-1 flex-col">
        {header}
        <div
          className={cx("flex min-h-0 flex-1 flex-col gap-3 pb-4", embedded ? "px-3 pt-3" : "px-5")}
        >
          {noTranscripts && agent.role !== "staff" ? (
            <p className="text-[12px] text-ink-3">{transcript?.error}</p>
          ) : null}
          {agent.state === "needs_trust" ? <TrustBanner agent={agent} path={cwd} /> : null}
          {agent.state === "needs_login" ? <LoginBanner agent={agent} /> : null}
          {approval ? <ApprovalBanner approval={approval} /> : null}
          <AgentTerminal key={agent.id} agentId={agent.id} alive={alive} className="flex-1" />
          <AgentFooter agent={agent} cwd={cwd} />
        </div>
      </div>
    );
  }

  return (
    <div className="flex min-h-0 flex-1">
      <section className="@container flex min-w-0 flex-1 flex-col" aria-label="Conversation">
        {header}
        {loading ? (
          <div className="flex flex-1 items-center justify-center">
            <Spinner />
          </div>
        ) : (
          <Conversation
            items={items}
            agentId={agent.id}
            inbox={noInbox}
            speaker={claude}
            working={
              agent.state === "working" || (agent.state === "starting" && pending.length > 0)
            }
            jumpSignal={jumps}
            onRetry={(p) => void cos.retryAgentPending(agent.id, p.id)}
            onDiscard={(p) => cos.discardAgentPending(agent.id, p.id)}
            empty={
              <AgentWelcome
                starting={agent.state === "starting"}
                error={transcript?.error ?? null}
                onTerminal={() => toggleTerminal(true)}
              />
            }
          />
        )}
        <div
          className={cx(
            "mx-auto flex w-full max-w-[740px] shrink-0 flex-col gap-2.5 pb-4",
            embedded ? "px-4" : "px-8",
          )}
        >
          {agent.state === "needs_trust" ? <TrustBanner agent={agent} path={cwd} /> : null}
          {agent.state === "needs_login" ? (
            <LoginBanner agent={agent} where="on the right" />
          ) : null}
          {approval ? <ApprovalBanner approval={approval} /> : null}
          <ChatComposer
            ref={input}
            label={`Message ${agent.title}`}
            placeholder={blocked ?? "Reply to Claude…"}
            value={draft}
            onChange={setDraft}
            onSend={send}
            sending={sending}
            disabled={!canChange || blocked !== null}
            files={canChange ? files : undefined}
            note={
              agent.state === "working" && draft.trim()
                ? "It's working: Claude reads this when its turn ends"
                : null
            }
          />
          <AgentFooter agent={agent} cwd={cwd} />
        </div>
      </section>

      {terminalOpen ? <SplitDivider label="Resize the agent's terminal" /> : null}
      {terminalOpen ? (
        <aside
          aria-label="Claude's terminal"
          className="flex w-[46%] max-w-full min-w-0 shrink-0 flex-col bg-page"
        >
          <div
            data-tauri-drag-region
            className="flex h-[52px] shrink-0 items-center gap-2 px-4 text-[12.5px] select-none"
          >
            <SquareTerminal className="size-4 text-ink-3" strokeWidth={1.8} aria-hidden />
            <span className="font-medium text-ink">Claude's terminal</span>
            <span className="truncate text-ink-3">· {agent.model ?? "default model"}</span>
            <IconButton
              icon={X}
              label="Hide the terminal"
              className="ml-auto"
              onClick={() => ui.toggleAgentTerminal(false)}
            />
          </div>
          <div className="flex min-h-0 flex-1 flex-col px-3 pb-4">
            <AgentTerminal
              key={agent.id}
              agentId={agent.id}
              alive={alive}
              autoFocus={agent.state === "needs_login"}
              className="flex-1"
            />
          </div>
        </aside>
      ) : null}
    </div>
  );
}

const noPending: PendingMessage[] = [];
const noInbox: InboxItem[] = [];

/** How an agent's Claude signs its turns. */
const claude: Speaker = {
  name: "Claude",
  mark: (
    <span className="flex size-5 shrink-0 items-center justify-center rounded-md bg-tone-blue/16 text-tone-blue">
      <Bot className="size-3" strokeWidth={2} aria-hidden />
    </span>
  ),
  nameClass: "text-tone-blue",
  dotClass: "bg-tone-blue",
};

/**
 * Why you can't type to the agent now, as the composer's placeholder; null
 * when you can. Text goes into its terminal, so it must not land on a
 * permission prompt, the trust dialog or the login screen.
 */
function composerBlock(state: Agent["state"]): string | null {
  switch (state) {
    case "starting":
      return "Claude is starting…";
    case "waiting_approval":
      return "Answer the request above first";
    case "needs_trust":
      return "Trust the folder first";
    case "needs_login":
      return "Log in to Claude first";
    case "exited":
    case "interrupted":
      return "It stopped. Resume it to send a message";
    default:
      return null;
  }
}

function AgentWelcome({
  starting,
  error,
  onTerminal,
}: {
  starting: boolean;
  error: string | null;
  onTerminal: () => void;
}) {
  return (
    <div className="flex flex-col items-start gap-2 pt-[9vh] pb-6">
      <h2 className="font-serif text-[22px] leading-tight font-medium text-ink">
        {starting ? "Claude is starting…" : "Nothing said yet"}
      </h2>
      <p className="max-w-[520px] text-[14px] leading-relaxed text-ink-2">
        Claude's replies show here as it works. Type below to give it a task, or{" "}
        <button
          type="button"
          onClick={onTerminal}
          className="rounded text-accent-ink underline-offset-2 hover:underline"
        >
          open its terminal
        </button>{" "}
        (Ctrl-Shift-J).
      </p>
      {error ? <p className="text-[13px] text-tone-red">{error}</p> : null}
    </div>
  );
}

export function TrustBanner({ agent, path }: { agent: Agent; path: string | null }) {
  const { cos, ui } = useApp();
  const canChange = useCanChange();
  const [busy, setBusy] = useState(false);
  return (
    <Banner icon={ShieldQuestion} tone="accent">
      <div className="min-w-0 flex-1">
        <p className="text-[13.5px] font-medium text-ink">
          Claude asks whether to trust this folder
        </p>
        {path ? <PathText path={path} /> : null}
      </div>
      <Button
        variant="accent"
        busy={busy}
        disabled={!canChange}
        onClick={() => {
          setBusy(true);
          void ui
            .perform("Couldn't answer the trust dialog", () => cos.trustAgent(agent.id))
            .finally(() => setBusy(false));
        }}
      >
        Trust folder
      </Button>
    </Banner>
  );
}

/**
 * Claude shows its login screen: nobody has logged into the agent's
 * CLAUDE_CONFIG_DIR. Logging in, in its terminal, lets it carry on. With
 * `onOpen` it offers to open the terminal; without, the terminal is `where`.
 */
export function LoginBanner({
  agent,
  onOpen,
  where = "below",
}: {
  agent: Agent;
  onOpen?: () => void;
  where?: string;
}) {
  return (
    <Banner icon={KeyRound} tone="accent">
      <div className="min-w-0 flex-1">
        <p className="text-[13.5px] font-medium text-ink">Claude needs you to log in</p>
        <LoginHint agent={agent} here={onOpen ? undefined : where} />
      </div>
      {onOpen ? (
        <Button variant="accent" icon={SquareTerminal} onClick={onOpen}>
          Open terminal
        </Button>
      ) : null}
    </Banner>
  );
}

/** "Open its terminal and log in, or run `CLAUDE_CONFIG_DIR=… claude` once." */
export function LoginHint({ agent, here }: { agent: Agent; here?: string }) {
  const dir = useCos((s) => configDirOf(s, agent));
  return (
    <p className="mt-0.5 text-[12.5px] leading-snug text-ink-2">
      {here ? `Log in in the terminal ${here}` : "Open its terminal and log in"}, or run{" "}
      <code
        className="rounded bg-subtle px-1 py-px font-mono text-[12px] text-ink select-all"
        title={dir ? `The account in ${dir} isn't logged in yet` : undefined}
      >
        {loginCommand(dir)}
      </code>{" "}
      once.
    </p>
  );
}

export function ApprovalBanner({ approval }: { approval: Approval }) {
  const why = routeExplanation(approval);
  return (
    <Banner icon={ShieldQuestion} tone="accent">
      <div className="min-w-0 flex-1">
        <p className="text-[13.5px] font-medium text-ink">{approvalHeadline(approval)}</p>
        <p
          className="mt-0.5 truncate font-mono text-[12.5px] text-ink-2 select-text"
          title={approvalDetail(approval)}
        >
          {approvalDetail(approval)}
        </p>
        {why ? <p className="mt-0.5 text-[12px] text-ink-3">{why}</p> : null}
      </div>
      <ApprovalButtons approval={approval} keys />
    </Banner>
  );
}

function Banner({
  icon: Icon,
  tone,
  children,
}: {
  icon: typeof Bot;
  tone: "accent" | "neutral";
  children: React.ReactNode;
}) {
  return (
    <div
      role="region"
      aria-label="Waiting"
      className={cx(
        "flex items-center gap-3.5 rounded-xl border px-4 py-3 animate-fade-in",
        tone === "accent" ? "border-accent/30 bg-accent/8" : "border-line bg-subtle/70",
      )}
    >
      <Icon
        className={cx("size-5 shrink-0", tone === "accent" ? "text-accent" : "text-ink-3")}
        strokeWidth={1.7}
      />
      {children}
    </div>
  );
}

/**
 * `env: CLAUDE_CONFIG_DIR, +2`, with every name in the tooltip: what cos set
 * for the agent at launch. Values are never shown here.
 */
/** The names an agent launched with; `bare` drops the leading dot, for a line of its own. */
export function EnvNames({ names, bare = false }: { names: string[]; bare?: boolean }) {
  const hint = envHint(names);
  if (!hint) return null;
  return (
    <span
      className="shrink-0 cursor-default"
      title={`Environment set at launch:\n${sortedNames(names).join("\n")}`}
    >
      {bare ? hint : `· ${hint}`}
    </span>
  );
}

function AgentFooter({ agent, cwd }: { agent: Agent; cwd: string | null }) {
  const parts = [
    agent.role !== "user" ? agent.role : null,
    agent.model ?? "default model",
    permissionModeTitle(agent.permissionMode),
    agent.pid !== null ? `pid ${agent.pid}` : null,
    agent.exitCode !== null ? `exit ${agent.exitCode}` : null,
  ].filter(Boolean);
  return (
    <div className="flex items-center gap-3 px-1 text-[12px] text-ink-3">
      <span className="shrink-0">{parts.join(" · ")}</span>
      <EnvNames names={agent.envNames} />
      {cwd ? <PathText path={cwd} className="ml-auto truncate text-[11.5px] text-ink-3" /> : null}
    </div>
  );
}

/**
 * Transcript or Terminal, for a session beside the chief. Ctrl-Shift-J flips
 * it from the chief's page.
 */
function ViewSwitch({
  terminal,
  onChange,
}: {
  terminal: boolean;
  onChange: (terminal: boolean) => void;
}) {
  const option = (on: boolean, value: boolean, icon: typeof MessageSquare, label: string) => {
    const Icon = icon;
    return (
      <button
        type="button"
        aria-pressed={on}
        title={`Show its ${label.toLowerCase()} (Ctrl-Shift-J)`}
        onClick={() => onChange(value)}
        className={cx(
          "flex h-6 items-center gap-1.5 rounded-md px-2 text-[11.5px] font-medium transition-colors outline-none focus-visible:ring-2 focus-visible:ring-accent/50",
          on ? "bg-surface text-ink shadow-card" : "text-ink-2 hover:text-ink",
        )}
      >
        <Icon className="size-3.5" strokeWidth={1.9} aria-hidden />
        {/* Icons only in a narrow pane, so the session's title keeps its room. */}
        <span className="sr-only @min-[34rem]:not-sr-only">{label}</span>
      </button>
    );
  };
  return (
    <div
      role="group"
      aria-label="Show the session as"
      className="mr-1 flex gap-0.5 rounded-lg bg-subtle p-0.5"
    >
      {option(!terminal, false, MessageSquare, "Transcript")}
      {option(terminal, true, SquareTerminal, "Terminal")}
    </div>
  );
}
