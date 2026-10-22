// The sidebar beside the rail: the machine picker, the project the rail
// picked (its chief, workspaces and agents), and which machine the app
// talks to.

import { useApp, useCos } from "../app/context";
import type { Project } from "../api/models";
import { HostPicker, HostStatus } from "./Hosts";
import { ProjectPanel } from "./sidebar/ProjectSection";

export function Sidebar({ project }: { project: Project | undefined }) {
  const { ui } = useApp();
  const connection = useCos((s) => s.connection);
  const projectsLoaded = useCos((s) => s.projectsLoaded);

  return (
    <aside className="flex w-[248px] shrink-0 flex-col bg-sidebar select-none" aria-label="Sidebar">
      {/* The window drags from the sidebar's top too. */}
      <div data-tauri-drag-region className="h-[5px] shrink-0" />
      <HostPicker />
      {project ? (
        <ProjectPanel key={project.id} project={project} />
      ) : (
        <div className="flex-1 px-4 py-2">
          <p className="text-[12.5px] leading-snug text-ink-3">
            {connection === "running" && projectsLoaded
              ? "No projects yet. Add a git repo to get started."
              : connection === "running"
                ? "Loading…"
                : "No projects to show."}
          </p>
          {connection === "running" && projectsLoaded ? (
            <button
              type="button"
              onClick={() => ui.open({ kind: "addProject" })}
              className="mt-3 rounded-lg border border-line-strong px-3 py-1.5 text-[12.5px] text-ink hover:bg-subtle"
            >
              Add project
            </button>
          ) : null}
        </div>
      )}
      <div className="shrink-0 border-t border-line px-2.5 py-2">
        <HostStatus />
      </div>
    </aside>
  );
}
