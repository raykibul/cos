// A project's environment variables for its agents: its Chief of Staff,
// the agents the chief starts and workspace agents. Opened from the
// project's ⋯ menu in the sidebar and on the chief's page.

import { FolderGit2 } from "lucide-react";
import { useApp, useCos } from "../app/context";
import { envNote } from "../domain/env";
import { project as projectOf } from "../store/selectors";
import { EmptyState } from "../ui/kit";
import { Crumb, CrumbSep, PageBody, PageHeader, PageTitle } from "../ui/Page";
import { EnvironmentList } from "./Environment";

export function ProjectEnvPage({ projectId }: { projectId: number }) {
  const { ui } = useApp();
  const project = useCos((s) => projectOf(s, projectId));
  const loaded = useCos((s) => s.projectsLoaded);

  if (!project) {
    return (
      <>
        <PageHeader />
        <PageBody>
          <EmptyState icon={FolderGit2} title={loaded ? "This project is gone" : "Loading…"}>
            {loaded ? "It was removed. Pick another one in the sidebar." : null}
          </EmptyState>
        </PageBody>
      </>
    );
  }

  return (
    <>
      <PageHeader>
        <Crumb onClick={() => ui.navigate({ kind: "chief", id: project.id })}>{project.name}</Crumb>
        <CrumbSep />
        <span className="truncate font-medium text-ink">Environment</span>
      </PageHeader>
      <PageBody>
        <PageTitle
          subtitle={
            <span className="block max-w-[620px] leading-relaxed">{envNote(project.name)}</span>
          }
        >
          Environment
        </PageTitle>
        <EnvironmentList projectId={project.id} heading="Variables" />
      </PageBody>
    </>
  );
}
