import { describe, expect, it } from "vitest";
import { fakeStore, settle } from "../test/harness";
import { projectInitial, projectShortcut } from "./rail";
import { filterItems, switchItems } from "./switcher";

async function items() {
  const { cos } = fakeStore();
  await cos.start();
  await settle();
  return switchItems(cos.get());
}

describe("switcher items", () => {
  it("lists each project's chief with its shortcut, then what waits on you", async () => {
    const all = await items();
    const chiefs = all.filter((i) => i.section === "Chiefs");
    expect(chiefs.map((i) => [i.title, i.shortcut, i.chiefOf])).toEqual([
      ["Chief of Staff · cos", "⌘1", 1],
      ["Chief of Staff · opal-app", "⌘2", 2],
    ]);
    const waiting = all.filter((i) => i.section === "Waiting on you");
    expect(waiting.some((i) => i.detail.startsWith("npm install left-pad --save"))).toBe(true);
    expect(waiting.some((i) => i.key === "login:11")).toBe(true);
    expect(all.find((i) => i.key === "events")?.page).toEqual({ kind: "events" });
  });

  it("shows a few of each section before you type, and no workspaces", async () => {
    const shown = filterItems(await items(), "");
    expect(shown.some((i) => i.section === "Workspaces")).toBe(false);
    expect(shown.filter((i) => i.section === "Waiting on you").length).toBeLessThanOrEqual(5);
  });

  it("matches every word, titles that start with the query first", async () => {
    const all = await items();
    expect(filterItems(all, "opal").map((i) => i.title)[0]).toBe("Chief of Staff · opal-app");
    expect(filterItems(all, "event log").map((i) => i.key)).toEqual(["events"]);
    expect(filterItems(all, "porto").some((i) => i.key === "workspace:4")).toBe(true);
    expect(filterItems(all, "zzz nothing")).toEqual([]);
  });
});

describe("rail names", () => {
  it("gives the first nine projects a shortcut", () => {
    expect(projectShortcut(0)).toBe("⌘1");
    expect(projectShortcut(8)).toBe("⌘9");
    expect(projectShortcut(9)).toBeNull();
  });

  it("puts the first letter or digit on the tile", () => {
    expect(projectInitial("Opal-app")).toBe("o");
    expect(projectInitial("_42-tools")).toBe("4");
    expect(projectInitial("--")).toBe("?");
  });
});
