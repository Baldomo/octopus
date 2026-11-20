import { createMemo, For, type JSX } from "solid-js";
import { createStore } from "solid-js/store";
import { backend } from "./backend.ts";
import { AGENT_PANEL, type PanelView, type PtyId, type TaskRef, type TaskView } from "./generated.ts";

/** The shell panel each task's tabs select on this client; the agent's tab is selected while none is. */
const [chosen, setChosen] = createStore<Record<string, PtyId | null>>({});
/** The shell panel each task's tabs select once the chosen one leaves; the agent's while none is. */
const [back, setBack] = createStore<Record<string, PtyId | null>>({});

/** Selects the shell panel's tab, or the agent's with `null`. */
export const chooseShell = (key: string, panel: PtyId | null) => {
  setChosen(key, panel);
  setBack(key, null);
};

/** Opens a shell panel at `path`, relative to the task's directory, and selects its tab. */
export const openShell = (at: TaskRef, key: string, path: string) =>
  backend.termOpen(at, path).then((panel) => chooseShell(key, panel));

/** This client holds the panel's terminal, where a program records with no virtual microphone to carry this client's. */
export const needsMicrophone = (panel: PanelView) =>
  panel.running && panel.held_here && panel.listening === "no_microphone";

export const microphonePrompt = (server: string) => `Voice needs the microphone on ${server}`;

/** Opens the panel installing the task's server's microphone and selects its tab, returning to `held` once it leaves. */
export const installMicrophone = (task: TaskView, held: PanelView) =>
  backend.termInstallMicrophone(task.at).then((panel) => {
    setChosen(task.key, panel);
    setBack(task.key, held.name === AGENT_PANEL ? null : held.id);
  });

/** The task's panels with the chosen shell selected, or the agent when no shell is chosen. */
export function taskPanels(task: () => TaskView | undefined) {
  const panels = () => task()?.panels ?? [];
  const agent = () => panels().find((panel) => panel.name === AGENT_PANEL);
  const shells = () => panels().filter((panel) => panel.name !== AGENT_PANEL);
  const shell = (id: PtyId | null | undefined) => shells().find((panel) => panel.id === id);
  const selected = () => shell(chosen[task()!.key]) ?? shell(back[task()!.key]) ?? agent();
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
