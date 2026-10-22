import { act, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";
import type { FakeDaemon } from "../bridge/fake";
import { fakeStore, settle } from "../test/harness";
import { App } from "./App";
import { createUiStore } from "./ui";

vi.mock("../terminal/engine", () => ({
  loadEngine: () => Promise.resolve(null),
  createEngine: () => Promise.resolve(null),
}));

async function renderSettings(prepare?: (daemon: FakeDaemon) => void) {
  const harness = fakeStore();
  prepare?.(harness.daemon);
  const ui = createUiStore({ page: { kind: "settings" } });
  render(<App cos={harness.cos} ui={ui} />);
  await act(async () => {
    await harness.cos.start();
    await settle();
  });
  return { ...harness, ui, user: userEvent.setup() };
}

const main = () => within(screen.getByRole("main"));
const section = () => within(main().getByRole("region", { name: "Judgment (jev)" }));

describe("Settings: Judgment (jev)", () => {
  it("shows where the key came from, the privacy note and the projects", async () => {
    await renderSettings();
    const key = within(section().getByRole("group", { name: "jev key" }));
    expect(key.getByText("Key found in your login shell")).toBeInTheDocument();
    expect(key.getByText("jev-latest · api.typesafe.ai")).toBeInTheDocument();
    expect(section().getByText(/never changes what happens/)).toBeInTheDocument();
    expect(section().getByText(/goes to TypeSafe/)).toBeInTheDocument();
    const projects = within(section().getByRole("list", { name: "Judged projects" }));
    expect(projects.getByRole("switch", { name: "Judge cos" })).toHaveAttribute(
      "aria-checked",
      "true",
    );
    expect(projects.getByRole("switch", { name: "Judge opal-app" })).toHaveAttribute(
      "aria-checked",
      "false",
    );
  });

  it("shows the permission and inbox stats with the disagreements", async () => {
    await renderSettings();
    const permission = within(section().getByRole("group", { name: "Permission requests" }));
    expect(
      permission.getByText("Would have auto-approved 41 of 45, disagreed on 4"),
    ).toBeInTheDocument();
    expect(permission.getByText("Asked you about 2 you then allowed")).toBeInTheDocument();
    expect(permission.getByText("52 decisions · 3 no opinion · 1 error")).toBeInTheDocument();
    const list = within(permission.getByRole("list", { name: "Disagreements" }));
    expect(list.getAllByRole("listitem")).toHaveLength(4);
    const first = within(list.getByRole("listitem", { name: "Bash rm -rf build" }));
    expect(first.getByText(/Would approve, you denied/)).toBeInTheDocument();
    expect(first.getByText("read_only (0.97), serves_task 0.91")).toBeInTheDocument();
    expect(first.getByText("14 min ago")).toBeInTheDocument();

    const inbox = within(section().getByRole("group", { name: "Chief inboxes" }));
    const count = (label: string) => inbox.getByText(label).nextElementSibling?.textContent;
    expect(count("Wake now")).toBe("7");
    expect(count("Batch")).toBe("15");
    expect(count("Let it be")).toBe("7");
    expect(inbox.getByText("31 decisions · 2 no opinion")).toBeInTheDocument();
  });

  it("shows empty states before anything is decided", async () => {
    await renderSettings((daemon) => {
      daemon.judge.points = [];
    });
    expect(
      within(section().getByRole("group", { name: "Permission requests" })).getByText(
        /Nothing judged yet/,
      ),
    ).toBeInTheDocument();
    expect(
      within(section().getByRole("group", { name: "Chief inboxes" })).getByText(
        /Nothing judged yet/,
      ),
    ).toBeInTheDocument();
  });

  it("switches a project on with PUT and shows it", async () => {
    const { user, daemon } = await renderSettings();
    const toggle = section().getByRole("switch", { name: "Judge opal-app" });
    await user.click(toggle);
    await act(() => settle());
    expect(
      daemon.requests.find((r) => r.method === "PUT" && r.path === "/v1/projects/2/judge")?.body,
    ).toEqual({ enabled: true });
    expect(daemon.judge.enabled.has(2)).toBe(true);
    expect(section().getByRole("switch", { name: "Judge opal-app" })).toHaveAttribute(
      "aria-checked",
      "true",
    );

    await user.click(section().getByRole("switch", { name: "Judge cos" }));
    await act(() => settle());
    expect(daemon.judge.enabled.has(1)).toBe(false);
    expect(section().getByRole("switch", { name: "Judge cos" })).toHaveAttribute(
      "aria-checked",
      "false",
    );
  });

  it("says why jev is off without a key, and checks again", async () => {
    const { user, daemon } = await renderSettings((d) => {
      d.judge.key = {
        source: "missing",
        detail: "~/.cos/daemon.env is readable by other users; not read",
        probing: false,
      };
    });
    const key = within(section().getByRole("group", { name: "jev key" }));
    expect(key.getByText("No key")).toBeInTheDocument();
    expect(key.getByText(/no TYPESAFE_API_KEY found/)).toBeInTheDocument();
    expect(key.getByText(/readable by other users/)).toBeInTheDocument();
    expect(section().getByText("On, waiting for a key")).toBeInTheDocument();

    await user.click(section().getByRole("button", { name: "Check again" }));
    await act(() => settle());
    expect(daemon.requests.some((r) => r.method === "POST" && r.path === "/v1/judge/reload")).toBe(
      true,
    );
    expect(key.getByText("Key found in your login shell")).toBeInTheDocument();
  });

  it("says when it's still checking the login shell", async () => {
    await renderSettings((d) => {
      d.judge.key = { source: "missing", detail: null, probing: true };
    });
    expect(section().getByText("Checking your login shell…")).toBeInTheDocument();
  });

  it("follows judge events from the daemon", async () => {
    const { daemon } = await renderSettings();
    await act(async () => {
      daemon.judge.enabled.add(2);
      daemon.log("judge.changed", { project_id: 2, enabled: true });
      await settle();
    });
    expect(section().getByRole("switch", { name: "Judge opal-app" })).toHaveAttribute(
      "aria-checked",
      "true",
    );
  });

  it("is hidden on a daemon without jev", async () => {
    await renderSettings((daemon) => {
      daemon.judge.route = () => null;
    });
    expect(main().queryByRole("region", { name: "Judgment (jev)" })).not.toBeInTheDocument();
    // The rest of Settings is there.
    expect(main().getByRole("region", { name: "Environment" })).toBeInTheDocument();
  });
});
