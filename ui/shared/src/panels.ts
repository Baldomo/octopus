import { createMemo, For, type JSX } from "solid-js";
import { createStore } from "solid-js/store";
import { backend } from "./backend.ts";
import { AGENT_PANEL, type PanelView, type PtyId, type TaskRef, type TaskView } from "./generated.ts";

/** The shell panel each task's tabs select on this client; the agent's tab is selected while none is. */
const [chosen, setChosen] = createStore<Record<string, PtyId | null>>({});

/** Selects the shell panel's tab, or the agent's with `null`. */
export const chooseShell = (key: string, panel: PtyId | null) => setChosen(key, panel);

/** Opens a shell panel at `path`, relative to the task's directory, and selects its tab. */
export const openShell = (at: TaskRef, key: string, path: string) =>
  backend.termOpen(at, path).then((panel) => chooseShell(key, panel));

/** The task's panels with the chosen shell selected, or the agent when no shell is chosen. */
export function taskPanels(task: () => TaskView | undefined) {
  const panels = () => task()?.panels ?? [];
  const agent = () => panels().find((panel) => panel.name === AGENT_PANEL);
  const shells = () => panels().filter((panel) => panel.name !== AGENT_PANEL);
  const selected = () => shells().find((panel) => panel.id === chosen[task()!.key]) ?? agent();
  const agentOn = () => selected() === agent();
  return { panels, agent, shells, selected, agentOn };
}

/** A child unmounting with its panel gone from the list still reads the panel as it last was. */
export function EachPanel(props: { panels: PanelView[]; children: (panel: () => PanelView) => JSX.Element }) {
  return For({
    get each() {
      return props.panels.map((panel) => panel.id);
    },
    children: (id) => {
      const panel = createMemo<PanelView>((last) => props.panels.find((each) => each.id === id) ?? last!);
      return props.children(panel);
    },
  });
}
