import { createRoot, createSelector, createSignal } from "solid-js";
import { createStore, reconcile } from "solid-js/store";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { backend } from "./backend";
import { FLOAT_LAYER, FLOAT_TOP } from "./generated";
import type {
  Boot,
  Bounds,
  Model,
  Modal,
  ServerView,
  Surface,
  TaskRef,
  TaskState,
  TaskView,
  Ui,
  UiAct,
  View,
} from "./generated";

export type Dragged = { key: string; at: TaskRef; section: string; state: TaskState };

export const [model, setModel] = createStore<Model>({
  version: -1,
  lists: { sections: [], workspaces: [] },
  tasks: {},
});
/** Which layer of the window this webview renders: base beneath the pages, or float above them. */
export const layer = getCurrentWebview().label === FLOAT_LAYER ? "float" : "base";

export const [servers, setServers] = createSignal<ServerView[]>([]);
export const [me, setMe] = createSignal("");

/** The state both layers render, owned by the app; every change goes through it and returns as a newer `Ui`. */
const [ui, setUi] = createStore<Ui>({
  version: -1,
  prefs: { pinned: false, collapsed: [], surface: "files", view: "task", open: null, board: null },
  recent: [],
  sidebar: false,
  modal: null,
  reveal: null,
  problem: null,
  viewport: { width: 0, height: 0 },
  float: { x: 0, y: FLOAT_TOP, width: 0, height: 0 },
});
export { ui };
/** The section the next new-task form opens on; the sidebar and the form share the float layer. */
export const [newTaskSection, setNewTaskSection] = createSignal<string | null>(null);
/** A drag lives inside the layer it started in. */
export const [dragged, setDragged] = createSignal<Dragged | null>(null);

/** A problem the app cannot take has nowhere left to show but the console. */
export const report = (error: unknown) => backend.uiAct({ do: "report", problem: String(error) }).catch(console.error);
const uiAct = (act: UiAct) => backend.uiAct(act).catch(report);

/** Lets each of a thousand rows track whether it is the open one without each re-running on a switch. */
export const isOpen = createRoot(() => createSelector(() => ui.prefs.open));

export const openTask = (key: string) => uiAct({ do: "open_task", key });
export const showView = (view: View) => uiAct({ do: "show_view", view });
export const showBoard = (section: string) => uiAct({ do: "show_board", section });
export const showSurface = (surface: Surface) => uiAct({ do: "show_surface", surface });
export const togglePin = () => uiAct({ do: "toggle_pin" });
export const toggleCollapsed = (section: string) => uiAct({ do: "toggle_collapsed", section });
export const showSidebar = (shown: boolean) => uiAct({ do: "show_sidebar", shown });
export const toggleSidebar = () => uiAct({ do: "toggle_sidebar" });
export const floatSurface = (surface: string, rect: Bounds, modal: boolean) =>
  uiAct({ do: "float", surface, rect, modal });
export const sinkSurface = (surface: string) => uiAct({ do: "sink", surface });
export const showModal = (modal: Modal | null) => uiAct({ do: "show_modal", modal });
export const reveal = (task: string, path: string) => uiAct({ do: "reveal", task, path });

export function act(promise: Promise<unknown>) {
  promise.catch(report);
}

/** The boot reply and events travel apart, so the newest version wins. */
export function applyModel(next: Model) {
  if (next.version > model.version) setModel(reconcile(next, { key: "key" }));
}

export function applyUi(next: Ui) {
  if (next.version > ui.version) setUi(reconcile(next));
}

export function applyBoot(boot: Boot) {
  setMe(boot.me);
  setServers(boot.servers);
  applyModel(boot.model);
  applyUi(boot.ui);
}

export const openView = (): TaskView | undefined => {
  const key = ui.prefs.open;
  return key === null ? undefined : model.tasks[key];
};

export const serverView = (name: string) => servers().find((server) => server.name === name);

export const isLocal = (name: string) => serverView(name)?.endpoint === null;

export function showTasks() {
  const section = openView()?.section ?? ui.prefs.board;
  if (section === null) showView("tasks");
  else showBoard(section);
}
