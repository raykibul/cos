// The tabs across the top of the window: every page you opened, the chief's
// and agents' among them, until you close it. A click selects a tab; × or a
// middle click closes it, as does ⌘W for the selected one. The bar's
// empty space drags the window.

import {
  GitBranch,
  PanelLeft,
  Inbox,
  LayoutGrid,
  NotebookPen,
  ScrollText,
  Settings,
  SquareTerminal,
  Terminal,
  Variable,
  X,
  type LucideIcon,
} from "lucide-react";
import { useApp, useCos, useUi } from "../app/context";
import { useSidebar } from "../app/layout";
import { tabInfo, tabState } from "../app/tabs";
import { type Page, samePage } from "../app/ui";
import { cx } from "../ui/cx";
import { StateDot } from "../ui/kit";
import { ChiefMark } from "./sidebar/ProjectSection";

const icons: Record<Exclude<Page["kind"], "chief">, LucideIcon> = {
  waiting: Inbox,
  events: ScrollText,
  settings: Settings,
  project: LayoutGrid,
  projectEnv: Variable,
  chiefMemory: NotebookPen,
  workspace: GitBranch,
  agent: SquareTerminal,
  shell: Terminal,
};

export function TabBar() {
  const { ui } = useApp();
  const tabs = useUi((s) => s.tabs);
  const sidebar = useSidebar();
  return (
    <div
      data-tauri-drag-region
      className="flex h-[46px] shrink-0 items-end gap-0.5 overflow-x-auto border-b border-line bg-sidebar px-2 select-none [scrollbar-width:none]"
    >
      <button
        type="button"
        aria-label={sidebar.shown ? "Hide the sidebar" : "Show the sidebar"}
        title={`${sidebar.shown ? "Hide" : "Show"} the sidebar (⌘B)`}
        aria-pressed={sidebar.shown}
        onClick={() => ui.toggleSidebar(sidebar.narrow)}
        className="mr-1 mb-[7px] flex size-7 shrink-0 items-center justify-center self-end rounded-lg text-ink-3 hover:bg-subtle hover:text-ink"
      >
        <PanelLeft className="size-4" strokeWidth={1.8} />
      </button>
      <div role="tablist" aria-label="Open pages" className="flex min-w-0 items-end gap-0.5">
        {tabs.map((page) => (
          <Tab key={`${page.kind}:${"id" in page ? page.id : ""}`} page={page} />
        ))}
      </div>
      <div data-tauri-drag-region className="h-full min-w-6 flex-1" />
    </div>
  );
}

function Tab({ page }: { page: Page }) {
  const { ui } = useApp();
  const selected = useUi((s) => samePage(s.page, page));
  const info = useCos((s) => tabInfo(s, page));
  const state = tabState(info);
  const Icon = page.kind === "chief" ? null : icons[page.kind];
  return (
    <div
      className={cx(
        "group relative flex h-[34px] max-w-[220px] min-w-[96px] shrink-0 items-center rounded-t-lg text-[12.5px] transition-colors",
        selected ? "bg-page font-medium text-ink" : "text-ink-2 hover:bg-subtle hover:text-ink",
      )}
    >
      <button
        type="button"
        role="tab"
        aria-selected={selected}
        title={info.hint}
        onClick={() => ui.navigate(page)}
        onAuxClick={(event) => {
          if (event.button === 1) ui.closeTab(page);
        }}
        className="flex h-full min-w-0 flex-1 items-center gap-2 rounded-t-lg pr-1 pl-3 text-left outline-none focus-visible:ring-2 focus-visible:ring-accent/50"
      >
        {page.kind === "chief" ? (
          <ChiefMark size="xs" />
        ) : state ? (
          <StateDot label={state} className="size-[7px]" />
        ) : Icon ? (
          <Icon className="size-3.5 shrink-0 text-ink-3" strokeWidth={1.9} aria-hidden />
        ) : null}
        <span className="truncate">{info.title}</span>
      </button>
      <button
        type="button"
        aria-label={`Close ${info.title}`}
        title="Close tab (⌘W)"
        onClick={() => ui.closeTab(page)}
        className={cx(
          "mr-1.5 flex size-5 shrink-0 items-center justify-center rounded text-ink-3 hover:bg-subtle-2 hover:text-ink",
          selected ? "opacity-100" : "opacity-0 group-hover:opacity-100 focus-visible:opacity-100",
        )}
      >
        <X className="size-3" strokeWidth={2.2} aria-hidden />
      </button>
    </div>
  );
}
