// What the app says about environment variables, shared by the lists, the
// editor and Settings.

/** The hint next to CLAUDE_CONFIG_DIR. */
export function accountHint(projectId: number | null): string {
  return projectId === null
    ? "Which Claude account agents use"
    : "Which Claude account this project's agents use";
}

/** Who gets a scope's variables, and when a change applies. */
export function envNote(projectName: string | null): string {
  const who =
    projectName === null
      ? "Used by every project's agents, unless a project sets its own value."
      : `Used by every agent in ${projectName}: its Chief of Staff, workspace agents and runs.`;
  return `${who} Running agents keep their current values until they restart.`;
}
