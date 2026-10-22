import { describe, expect, it } from "vitest";
import { MAX_TABS, type Page, closeTab, createUiStore, openTab } from "./ui";

const chief = (id: number): Page => ({ kind: "chief", id });
const agent = (id: number): Page => ({ kind: "agent", id });
const waiting: Page = { kind: "waiting" };

describe("tabs", () => {
  it("start with the first page", () => {
    const ui = createUiStore({ page: chief(1) });
    expect(ui.store.getState().tabs).toEqual([chief(1)]);
  });

  it("open a new page right of the selected tab, and reuse the tab a page already has", () => {
    expect(openTab([chief(1), chief(2)], chief(1), agent(5))).toEqual([
      chief(1),
      agent(5),
      chief(2),
    ]);
    const tabs = [chief(1), agent(5)];
    expect(openTab(tabs, agent(5), chief(1))).toBe(tabs);
  });

  it("drop the oldest tab you aren't on past the limit", () => {
    const full = Array.from({ length: MAX_TABS }, (_, i) => agent(i + 1));
    const next = openTab(full, agent(MAX_TABS), chief(1));
    expect(next).toHaveLength(MAX_TABS);
    expect(next[0]).toEqual(agent(2));
    expect(next.at(-1)).toEqual(chief(1));
  });

  it("select the right neighbour when the selected tab closes, else the left one", () => {
    const tabs = [chief(1), agent(5), chief(2)];
    expect(closeTab(tabs, agent(5), agent(5))).toEqual({
      tabs: [chief(1), chief(2)],
      page: chief(2),
    });
    expect(closeTab(tabs, chief(2), chief(2))).toEqual({
      tabs: [chief(1), agent(5)],
      page: agent(5),
    });
    // Closing another tab keeps the selection.
    expect(closeTab(tabs, chief(2), chief(1))).toEqual({
      tabs: [agent(5), chief(2)],
      page: chief(2),
    });
  });

  it("fall back to Waiting on you when the last one closes", () => {
    expect(closeTab([chief(1)], chief(1), chief(1))).toEqual({ tabs: [waiting], page: waiting });
  });

  it("navigate, close and cycle through the store", () => {
    const ui = createUiStore({ page: waiting });
    ui.navigate(chief(1));
    ui.navigate(agent(3));
    expect(ui.store.getState().tabs).toEqual([waiting, chief(1), agent(3)]);
    ui.navigate(waiting);
    expect(ui.store.getState().tabs).toHaveLength(3);
    expect(ui.store.getState().page).toEqual(waiting);
    ui.cycleTab(-1);
    expect(ui.store.getState().page).toEqual(agent(3));
    ui.cycleTab(1);
    expect(ui.store.getState().page).toEqual(waiting);
    ui.closeTab();
    expect(ui.store.getState()).toMatchObject({ page: chief(1), tabs: [chief(1), agent(3)] });
  });
});
