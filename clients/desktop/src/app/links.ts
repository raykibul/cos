// Links leave the app. Clicking a web or mail link (in a chat's markdown, a
// transcript) or ⌘-clicking a URL in a terminal (ghostty-web calls
// `window.open`) opens it in the default browser, never inside the window.
// The shell's `links` plugin backs this up for any other navigation.

const OUTSIDE = new Set(["http:", "https:", "mailto:"]);

/**
 * `href`, resolved against the page at `base`, when it's a web or mail link
 * away from the app; null for the app's own pages and anything else.
 */
export function externalUrl(href: string, base: string): string | null {
  let url: URL;
  try {
    url = new URL(href, base);
  } catch {
    return null;
  }
  if (!OUTSIDE.has(url.protocol)) return null;
  if (url.protocol !== "mailto:" && url.origin === new URL(base).origin) return null;
  return url.href;
}

/**
 * Sends the window's outgoing links to `open` instead of following them.
 * Returns a function that undoes it.
 */
export function openLinksOutside(win: Window, open: (url: string) => void): () => void {
  const onClick = (event: MouseEvent) => {
    if (event.defaultPrevented || event.button > 1) return;
    const target = event.target;
    const anchor = target && "closest" in target ? (target as Element).closest("a[href]") : null;
    if (!anchor) return;
    const url = externalUrl(anchor.getAttribute("href") ?? "", win.location.href);
    if (!url) return;
    event.preventDefault();
    open(url);
  };
  win.document.addEventListener("click", onClick);
  win.document.addEventListener("auxclick", onClick);

  const windowOpen = win.open;
  win.open = (target?: string | URL, ...rest: [string?, string?]) => {
    const url = target === undefined ? null : externalUrl(String(target), win.location.href);
    if (!url) return windowOpen.call(win, target, ...rest);
    open(url);
    return null;
  };

  return () => {
    win.document.removeEventListener("click", onClick);
    win.document.removeEventListener("auxclick", onClick);
    win.open = windowOpen;
  };
}
