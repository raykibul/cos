import { useSyncExternalStore } from "react";

export type Scheme = "light" | "dark";

/** `data-theme` on <html> wins; otherwise the system's setting. */
export function currentScheme(): Scheme {
  if (typeof document === "undefined") return "light";
  const forced = document.documentElement.dataset.theme;
  if (forced === "light" || forced === "dark") return forced;
  if (typeof window.matchMedia !== "function") return "light";
  return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}

function subscribe(listener: () => void): () => void {
  const media =
    typeof window.matchMedia === "function"
      ? window.matchMedia("(prefers-color-scheme: dark)")
      : null;
  media?.addEventListener("change", listener);
  const observer = new MutationObserver(listener);
  observer.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
  return () => {
    media?.removeEventListener("change", listener);
    observer.disconnect();
  };
}

export function useScheme(): Scheme {
  return useSyncExternalStore(subscribe, currentScheme, () => "light");
}
