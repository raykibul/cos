import { describe, expect, it } from "vitest";
import { MAX_TABS, type Page, allTabs, closeTab, createUiStore, enterProject, openTab } from "./ui";

const chief = (id: number): Page => ({ kind: "chief", id });
const agent = (id: number): Page => ({ kind: "agent", id });
const waiting: Page = { kind: "waiting" };
const events: Page = { kind: "events" };

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

describe("tabs per project", () => {
  it("move to another project's tabs, keeping the ones you leave", () => {
    const ui = createUiStore({ page: chief(1) });
    ui.navigate(events);
    ui.navigate(chief(2));
    expect(ui.store.getState()).toMatchObject({
      project: 2,
      tabs: [chief(2)],
      page: chief(2),
      saved: { 1: { tabs: [chief(1), events], page: events } },
    });
    ui.navigate(chief(1));
    expect(ui.store.getState()).toMatchObject({
      project: 1,
      tabs: [chief(1), events],
      page: chief(1),
      saved: { 2: { tabs: [chief(2)], page: chief(2) } },
    });
  });

  it("open pages of no project in the project you're in", () => {
    const ui = createUiStore({ page: chief(1) });
    ui.navigate(chief(2));
    ui.navigate(waiting);
    expect(ui.store.getState()).toMatchObject({ project: 2, tabs: [chief(2), waiting] });
  });

  it("go back to a project's last tab, or its chief when you're there or new to it", () => {
    const ui = createUiStore({ page: chief(1) });
    ui.navigate(events);
    ui.openProject(2);
    expect(ui.store.getState().page).toEqual(chief(2));
    ui.openProject(1);
    expect(ui.store.getState().page).toEqual(events);
    ui.openProject(1);
    expect(ui.store.getState().page).toEqual(chief(1));
  });

  it("place an agent by the daemon's state", () => {
    const ui = createUiStore({ page: chief(1) });
    ui.setProjectOf((page) => (page.kind === "agent" ? 2 : page.kind === "chief" ? page.id : null));
    ui.navigate(agent(5));
    expect(ui.store.getState()).toMatchObject({ project: 2, tabs: [agent(5)] });
  });

  it("move a page whose project was found late into that project", () => {
    const ui = createUiStore({ page: chief(1) });
    ui.showProject(1);
    ui.navigate(agent(5)); // Not known yet: stays in project 1.
    expect(ui.store.getState().tabs).toEqual([chief(1), agent(5)]);
    ui.setProjectOf((page) => (page.kind === "agent" ? 2 : page.kind === "chief" ? page.id : null));
    ui.showProject(2);
    expect(ui.store.getState()).toMatchObject({
      project: 2,
      tabs: [agent(5)],
      saved: { 1: { tabs: [chief(1)], page: chief(1) } },
    });
  });

  it("adopt the first tabs for the first project shown", () => {
    const ui = createUiStore({ page: waiting });
    ui.navigate(events);
    ui.showProject(1);
    expect(ui.store.getState()).toMatchObject({ project: 1, tabs: [waiting, events], saved: {} });
  });

  it("close another project's tab", () => {
    const ui = createUiStore({ page: chief(1) });
    ui.navigate(events);
    ui.navigate(chief(2));
    ui.closeTab(events);
    expect(ui.store.getState().saved[1]).toEqual({ tabs: [chief(1)], page: chief(1) });
    expect(allTabs(ui.store.getState())).toEqual([chief(2), chief(1)]);
  });

  it("forget a removed project's tabs", () => {
    const ui = createUiStore({ page: chief(1) });
    ui.navigate(chief(2));
    ui.navigate(waiting);
    ui.forgetProject(1);
    expect(ui.store.getState().saved).toEqual({});
    ui.forgetProject(2);
    expect(ui.store.getState()).toMatchObject({ project: null, tabs: [waiting], page: waiting });
  });

  it("keep a page of no project in both when it goes along", () => {
    const s = { project: 1, tabs: [chief(1), waiting], page: waiting, saved: {} };
    expect(enterProject(s, 2, waiting, false)).toEqual({
      project: 2,
      tabs: [waiting],
      page: waiting,
      saved: { 1: { tabs: [chief(1), waiting], page: waiting } },
    });
  });
});
