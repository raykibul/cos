// How the project rail names its projects: a letter for the tile and the
// shortcut that opens the chief.

/** ⌘1 to ⌘9 for the first nine projects. */
export function projectShortcut(index: number): string | null {
  return index < 9 ? `⌘${index + 1}` : null;
}

/** The letter on a project's tile: its first letter or digit, lowercase. */
export function projectInitial(name: string): string {
  const letter = [...name].find((c) => /[\p{L}\p{N}]/u.test(c));
  return (letter ?? "?").toLowerCase();
}
