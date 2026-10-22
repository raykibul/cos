// Terminal colours that sit with the app: a warm background a step off the
// page's (white on light, the sidebar's near-black on dark), a
// Claude-orange cursor, and an ANSI palette that reads on it. Claude Code
// draws mostly in 24-bit colour, so the palette matters for the rest.

export interface TerminalTheme {
  foreground: string;
  background: string;
  cursor: string;
  cursorAccent: string;
  selectionBackground: string;
  selectionForeground?: string;
  black: string;
  red: string;
  green: string;
  yellow: string;
  blue: string;
  magenta: string;
  cyan: string;
  white: string;
  brightBlack: string;
  brightRed: string;
  brightGreen: string;
  brightYellow: string;
  brightBlue: string;
  brightMagenta: string;
  brightCyan: string;
  brightWhite: string;
}

export const darkTheme: TerminalTheme = {
  foreground: "#ECEAE3",
  background: "#1F1E1D",
  cursor: "#D97757",
  cursorAccent: "#1F1E1D",
  selectionBackground: "#5A4237",
  black: "#3A3936",
  red: "#E5786D",
  green: "#8FC08B",
  yellow: "#E8C07D",
  blue: "#86A8E8",
  magenta: "#C49BDB",
  cyan: "#7EC4C1",
  white: "#D6D3CB",
  brightBlack: "#7D7A72",
  brightRed: "#F08F85",
  brightGreen: "#A8D4A4",
  brightYellow: "#F2D29A",
  brightBlue: "#A0BDF0",
  brightMagenta: "#D4B1E6",
  brightCyan: "#9AD6D3",
  brightWhite: "#FAF9F5",
};

/** Yellow and white are darkened so they stay readable on the light page. */
export const lightTheme: TerminalTheme = {
  foreground: "#1F1E1D",
  background: "#FFFFFF",
  cursor: "#C6613F",
  cursorAccent: "#FFFFFF",
  selectionBackground: "#F0D8CC",
  black: "#1F1E1D",
  red: "#B5463C",
  green: "#3F7D45",
  yellow: "#946612",
  blue: "#2F5FB3",
  magenta: "#8A4CA8",
  cyan: "#237A78",
  white: "#6F6D66",
  brightBlack: "#8F8D86",
  brightRed: "#CC5A4F",
  brightGreen: "#4E9455",
  brightYellow: "#A9781A",
  brightBlue: "#4373C9",
  brightMagenta: "#9E5FBD",
  brightCyan: "#2E8E8B",
  brightWhite: "#3D3C39",
};

export const terminalFont = '"JetBrains Mono Variable", "SF Mono", Menlo, Monaco, monospace';
export const terminalFontSize = 13;
