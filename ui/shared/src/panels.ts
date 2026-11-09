import { createMemo, For, type JSX } from "solid-js";
import { AGENT_PANEL, type PanelView, type PtyId, type TaskView } from "./generated.ts";

/** The task's panels with the chosen shell selected, or the agent when no shell is chosen. */
export function taskPanels(task: () => TaskView | undefined, chosen: () => PtyId | null) {
  const panels = () => task()?.panels ?? [];
  const agent = () => panels().find((panel) => panel.name === AGENT_PANEL);
  const shells = () => panels().filter((panel) => panel.name !== AGENT_PANEL);
  const selected = () => shells().find((panel) => panel.id === chosen()) ?? agent();
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
