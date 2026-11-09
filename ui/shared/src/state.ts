import { createRoot, createSelector, createSignal } from "solid-js";
import { createStore, reconcile } from "solid-js/store";
import { backend } from "./backend";
import {
  LIVE_STATES,
  type AccountView,
  type Appearance,
  type Boot,
  type Model,
  type Modal,
  type Prefs,
  type Section,
  type ServerView,
  type Surface,
  type TaskState,
  type TaskView,
  type Ui,
  type UiAct,
  type View,
  type ZoomStep,
} from "./generated";
import { inSectionOrder } from "./sectionOrder";

export const [model, setModel] = createStore<Model>({
  version: -1,
  lists: { sections: [], workspaces: [] },
  tasks: {},
});

/** The order a section being dragged shows the sections in, which stands over the saved one until it lands. */
export const [arranged, arrange] = createSignal<string[] | null>(null);
const sectionOrder = () => arranged() ?? ui.prefs.section_order;
export const sections = () => inSectionOrder(model.lists.sections, sectionOrder());
export const workspaces = () => inSectionOrder(model.lists.workspaces, sectionOrder());

export const [servers, setServers] = createSignal<ServerView[]>([]);
export const [me, setMe] = createSignal("");
export const [myKey, setMyKey] = createSignal("");
export const [account, setAccount] = createSignal<AccountView>({ id: "", member: true, devices: [] });

/** The state both layers render, owned by the app; every change goes through it and returns as a newer `Ui`. */
const [ui, setUi] = createStore<Ui>({
  version: -1,
  prefs: {
    pinned: false,
    collapsed: [],
    section_order: [],
    surface: "files",
    split: 0.5,
    view: "task",
    open: null,
    board: null,
    appearance: "system",
    zoom: 100,
    agent_flags: {},
  },
  recent: [],
  sidebar: false,
  client_menu: false,
  modal: null,
  reveal: null,
  problem: null,
  viewport: { width: 0, height: 0 },
  float_input: [],
});
export { ui };
/** The section the next new-task form opens on; the sidebar and the form share the float layer. */
export const [newTaskSection, setNewTaskSection] = createSignal<string | null>(null);

/** A problem the app cannot take has nowhere left to show but the console. */
export const report = (error: unknown) => backend.uiAct({ do: "report", problem: String(error) }).catch(console.error);
export const uiAct = (act: UiAct) => backend.uiAct(act).catch(report);

/** Lets each of a thousand rows track whether it is the open one without each re-running on a switch. */
export const isOpen = createRoot(() => createSelector(() => ui.prefs.open));

export const openTask = (key: string) => uiAct({ do: "open_task", key });
export const showView = (view: View) => uiAct({ do: "show_view", view });
export const showBoard = (section: string) => uiAct({ do: "show_board", section });
export const showSurface = (surface: Surface) => uiAct({ do: "show_surface", surface });
export const resetSplit = () => uiAct({ do: "reset_split" });
export const togglePin = () => uiAct({ do: "toggle_pin" });
export const toggleCollapsed = (section: string) => uiAct({ do: "toggle_collapsed", section });
export const orderSections = (names: string[]) => uiAct({ do: "order_sections", sections: names });
export const showSidebar = (shown: boolean) => uiAct({ do: "show_sidebar", shown });
export const toggleSidebar = () => uiAct({ do: "toggle_sidebar" });
export const showClientMenu = (shown: boolean) => uiAct({ do: "show_client_menu", shown });
export const toggleClientMenu = () => uiAct({ do: "toggle_client_menu" });
export const setAppearance = (appearance: Appearance) => uiAct({ do: "set_appearance", appearance });
export const zoom = (step: ZoomStep) => uiAct({ do: "zoom", step });
export const setAgentFlags = (harness: string, flags: string) =>
  backend.uiAct({ do: "set_agent_flags", harness, flags });

export const harnesses = () =>
  [
    ...new Set([...Object.values(model.tasks).map((view) => view.task.harness), ...Object.keys(ui.prefs.agent_flags)]),
  ].sort();
export const showModal = (modal: Modal | null) => uiAct({ do: "show_modal", modal });
export const reveal = (task: string, path: string) => uiAct({ do: "reveal", task, path });

export function act(promise: Promise<unknown>) {
  promise.catch(report);
}

/** The boot reply and events travel apart, so the newest version wins. */
export function applyModel(next: Model) {
  if (next.version > model.version) setModel(reconcile(next, { key: "key" }));
}

/** How many points one CSS pixel of the UI takes. */
export const scale = (prefs: Prefs) => prefs.zoom / 100;

/** Zooms the layout before the state lands, so whatever the new state measures, it measures at the new zoom. */
export function applyUi(next: Ui) {
  if (next.version <= ui.version) return;
  document.documentElement.style.setProperty("--zoom", String(scale(next.prefs)));
  setUi(reconcile(next));
}

export function applyBoot(boot: Boot) {
  setMe(boot.me);
  setMyKey(boot.key);
  setAccount(boot.account);
  setServers(boot.servers);
  applyModel(boot.model);
  applyUi(boot.ui);
}

export const openView = (): TaskView | undefined => {
  const key = ui.prefs.open;
  return key === null ? undefined : model.tasks[key];
};

export const serverView = (name: string) => servers().find((server) => server.name === name);

export const sectionView = (name: string | null | undefined) =>
  model.lists.sections.find((section) => section.name === name);

export const tasksOn = (section: Section, server: string) =>
  section.groups.reduce((count, group) => count + group.rows.filter((row) => row.at.server === server).length, 0);

export const isLive = (state: TaskState) => LIVE_STATES.some((live) => live === state);

export const isLocal = (name: string) => serverView(name)?.relay === null;

export const serverLabel = (name: string) => serverView(name)?.label ?? name;

export function showTasks() {
  const section = openView()?.section ?? ui.prefs.board;
  if (section === null) showView("tasks");
  else showBoard(section);
}
