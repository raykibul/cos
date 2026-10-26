import { afterEach, describe, expect, it, vi } from "vitest";
import { externalUrl, openLinksOutside } from "./links";

const app = "tauri://localhost/index.html";
const dev = "http://localhost:5173/?page=waiting";

describe("externalUrl", () => {
  it("sends web and mail links out", () => {
    expect(externalUrl("https://github.com/raykibul/cos", app)).toBe(
      "https://github.com/raykibul/cos",
    );
    expect(externalUrl("http://localhost:3000/", dev)).toBe("http://localhost:3000/");
    expect(externalUrl("mailto:someone@example.com", app)).toBe("mailto:someone@example.com");
  });

  it("keeps the app's own pages and other schemes in", () => {
    expect(externalUrl("#top", app)).toBeNull();
    expect(externalUrl("/?page=events", dev)).toBeNull();
    expect(externalUrl("http://localhost:5173/x", dev)).toBeNull();
    expect(externalUrl("file:///etc/hosts", app)).toBeNull();
    expect(externalUrl("javascript:void(0)", app)).toBeNull();
    expect(externalUrl("http://[bad", app)).toBeNull();
  });
});

describe("openLinksOutside", () => {
  let undo: (() => void) | undefined;
  afterEach(() => {
    undo?.();
    document.body.innerHTML = "";
  });

  function link(href: string): HTMLElement {
    document.body.innerHTML = `<p><a href="${href}"><strong>the link</strong></a></p>`;
    return document.querySelector("strong")!;
  }

  it("opens a clicked web link outside instead of following it", () => {
    const open = vi.fn();
    undo = openLinksOutside(window, open);
    const event = new MouseEvent("click", { bubbles: true, cancelable: true });
    link("https://example.com/docs").dispatchEvent(event);
    expect(open).toHaveBeenCalledWith("https://example.com/docs");
    expect(event.defaultPrevented).toBe(true);
  });

  it("leaves in-app links and handled clicks alone", () => {
    const open = vi.fn();
    undo = openLinksOutside(window, open);
    const inApp = new MouseEvent("click", { bubbles: true, cancelable: true });
    link("#section").dispatchEvent(inApp);
    expect(inApp.defaultPrevented).toBe(false);

    const handled = new MouseEvent("click", { bubbles: true, cancelable: true });
    handled.preventDefault();
    link("https://example.com/").dispatchEvent(handled);
    expect(open).not.toHaveBeenCalled();
  });

  it("opens terminal links (window.open) outside", () => {
    const open = vi.fn();
    const original = vi.fn(() => null);
    window.open = original;
    undo = openLinksOutside(window, open);
    expect(window.open("https://example.com/", "_blank", "noopener,noreferrer")).toBeNull();
    expect(open).toHaveBeenCalledWith("https://example.com/");
    expect(original).not.toHaveBeenCalled();
    undo();
    undo = undefined;
    expect(window.open).toBe(original);
  });
});
