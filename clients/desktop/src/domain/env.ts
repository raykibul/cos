// What the app says about environment variables, shared by the lists, the
// editor and Settings.

/** The hint next to CLAUDE_CONFIG_DIR. */
export function accountHint(projectId: number | null): string {
  return projectId === null
    ? "Which Claude account agents use"
    : "Which Claude account this project's agents use";
}

/**
 * What logs a config dir in from a shell: `CLAUDE_CONFIG_DIR=~/.claude-personal claude`
 * (the shell expands the `~`), or plain `claude` when cos sets no dir.
 */
export function loginCommand(configDir: string | null): string {
  if (!configDir) return "claude";
  // A leading ~/ stays outside quotes, so the shell still expands it.
  const home = configDir.startsWith("~/") ? "~/" : "";
  const rest = configDir.slice(home.length);
  const quoted =
    rest === "" || /^[A-Za-z0-9_./+-]+$/.test(rest) ? rest : `'${rest.replaceAll("'", `'\\''`)}'`;
  return `CLAUDE_CONFIG_DIR=${home}${quoted} claude`;
}

/** Who gets a scope's variables, and when a change applies. */
export function envNote(projectName: string | null): string {
  const who =
    projectName === null
      ? "Used by every project's agents, unless a project sets its own value."
      : `Used by every agent in ${projectName}: its Chief of Staff, workspace agents and runs.`;
  return `${who} Running agents keep their current values until they restart.`;
}
