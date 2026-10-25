// A shell's tab: the terminal Ctrl-J opened in a project's root or a
// session's folder, filling the page. Closing the tab ends the shell.

import { Terminal } from "lucide-react";
import { useCos } from "../app/context";
import type { Page } from "../app/ui";
import { project } from "../store/selectors";
import { AgentTerminal } from "../terminal/AgentTerminal";
import { PathText } from "../ui/kit";
import { PageHeader } from "../ui/Page";

export function ShellPage({ page }: { page: Extract<Page, { kind: "shell" }> }) {
  const projectName = useCos((s) =>
    page.projectId !== null ? (project(s, page.projectId)?.name ?? null) : null,
  );
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <PageHeader>
        <Terminal className="size-4 shrink-0 text-ink-3" strokeWidth={1.8} aria-hidden />
        <h1 className="shrink-0 font-medium text-ink">Terminal</h1>
        {projectName ? <span className="shrink-0">· {projectName}</span> : null}
        <span className="min-w-0 truncate">
          · <PathText path={page.cwd} className="break-normal" />
        </span>
      </PageHeader>
      <div className="flex min-h-0 flex-1 flex-col px-5 pb-4">
        <AgentTerminal key={page.id} agentId={page.id} kind="shell" alive className="flex-1" />
      </div>
    </div>
  );
}
