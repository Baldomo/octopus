import { createEffect, createMemo, createSignal, Match, on, onCleanup, Show, Switch } from "solid-js";
import { backend } from "./backend";
import { Board } from "./Board";
import { Browser, forgetBrowsersExcept } from "./Browser";
import { Files } from "./Files";
import { filesOf, forgetFilesExcept } from "./taskFiles";
import { commands, hint } from "./keys";
import { NewTask } from "./NewTask";
import { PairServer } from "./PairServer";
import { Palette } from "./Palette";
import { Sidebar } from "./Sidebar";
import { holdsKeyboard } from "./SidebarControls";
import { act, floatSurface, model, openView, serverView, showModal, showSidebar, sinkSurface, ui } from "./state";
import { StatusBar } from "./StatusBar";
import { TerminalStacks } from "./Terminals";
import { TitleBar } from "./TitleBar";
import { FLOAT_TOP, type Modal as ModalName } from "./generated";

/** This webview's own size, which grows only once the app has placed it and its renderer has taken the new size. */
const [webview, setWebview] = createSignal({ width: innerWidth, height: innerHeight });
addEventListener("resize", () => setWebview({ width: innerWidth, height: innerHeight }));

/**
 * A surface of the float layer: while it is `out`, and until its exit transition ends, the app covers its rect with
 * the float layer, and a surface holds the keyboard while it is `modal`. It is `shown` from the moment the webview holds it, so it
 * never draws where the window cannot show it yet.
 */
function floating(surface: { name: string; modal: () => boolean; out: () => boolean }) {
  const { name, modal, out } = surface;
  /** The surface's far corner in this webview's coordinates. */
  const [reach, setReach] = createSignal<{ right: number; bottom: number } | null>(null);
  const ref = (element: HTMLElement) => {
    createEffect(
      on(
        () => [out(), modal(), ui.viewport.width, ui.viewport.height] as const,
        ([isOut, isModal]) => {
          const rect = {
            x: element.offsetLeft,
            y: element.offsetTop,
            width: element.offsetWidth,
            height: element.offsetHeight,
          };
          setReach({ right: rect.x + rect.width, bottom: rect.y + rect.height - FLOAT_TOP });
          if (isOut) floatSurface(name, rect, isModal);
          else if (element.getAnimations().length === 0) sinkSurface(name);
        },
      ),
    );
    element.addEventListener("transitionend", (event) => {
      if (event.target === element && !out()) sinkSurface(name);
    });
    onCleanup(() => sinkSurface(name));
  };
  const held = () => {
    const corner = reach();
    return corner !== null && corner.right <= webview().width && corner.bottom <= webview().height;
  };
  const shown = createMemo<boolean>((was) => out() && (was || held()), false);
  return { ref, shown };
}

/** The base layer: docked UI takes the window's layout space, and each browser page sits in the rect it leaves. */
export function Docked() {
  createEffect(
    on(
      () => [...ui.recent],
      (keys) => {
        forgetFilesExcept(keys);
        forgetBrowsersExcept(keys);
      },
    ),
  );
  /** The open task is focused whenever it is in the Task view with its server reachable, so a restart refocuses it. */
  const shownInTaskView = createMemo(() => {
    const shown = ui.prefs.view === "task" ? openView() : undefined;
    return shown !== undefined && serverView(shown.at.server)?.reachable ? ui.prefs.open : null;
  });
  createEffect(
    on(shownInTaskView, (key) => {
      const shown = key === null ? undefined : openView();
      if (shown !== undefined) act(backend.taskFocus(shown.at));
    }),
  );
  createEffect(
    on(
      () => ui.reveal?.nth,
      () => {
        const revealed = ui.reveal;
        const task = revealed === null ? undefined : model.tasks[revealed.task];
        if (revealed !== null && task !== undefined) act(filesOf(task).open(revealed.path));
      },
      { defer: true },
    ),
  );

  return (
    <div class="app">
      <TitleBar />
      <div class="halves" style={{ display: ui.prefs.view === "task" ? "" : "none" }}>
        <div class="pane">
          <div class="tabs">
            <button class="tab" classList={{ on: ui.prefs.surface === "files" }} onClick={commands.files.run}>
              Files<kbd>{hint(commands.files)}</kbd>
            </button>
            <button class="tab" classList={{ on: ui.prefs.surface === "browser" }} onClick={commands.browser.run}>
              Browser<kbd>{hint(commands.browser)}</kbd>
            </button>
          </div>
          <Show when={openView()} fallback={<div class="empty">No task is open.</div>}>
            <div class="surface" style={{ display: ui.prefs.surface === "files" ? "" : "none" }}>
              <Files />
            </div>
            <div class="surface" style={{ display: ui.prefs.surface === "browser" ? "" : "none" }}>
              <Browser visible={ui.prefs.view === "task" && ui.prefs.surface === "browser"} />
            </div>
          </Show>
        </div>
        <TerminalStacks />
      </div>
      <div class="tasks-view" style={{ display: ui.prefs.view === "tasks" ? "" : "none" }}>
        <Board />
      </div>
      <StatusBar />
      <div class="edge" onPointerEnter={() => showSidebar(true)} />
    </div>
  );
}

function Modal(props: { modal: ModalName }) {
  const scrim = floating({ name: "modal", modal: () => true, out: () => true });
  return (
    <>
      <div class="scrim" ref={scrim.ref} onClick={() => showModal(null)} />
      <Show when={scrim.shown()}>
        <Switch>
          <Match when={props.modal === "palette"}>
            <Palette />
          </Match>
          <Match when={props.modal === "new_task"}>
            <NewTask />
          </Match>
          <Match when={props.modal === "pair_server"}>
            <PairServer />
          </Match>
        </Switch>
      </Show>
    </>
  );
}

/**
 * The float layer: floating UI draws over the browser pages. It lays out in window coordinates, and the app grows the
 * layer from a corner that never moves to cover whatever floats.
 */
export function Floating() {
  const sidebar = floating({ name: "sidebar", modal: holdsKeyboard, out: () => ui.sidebar });
  return (
    <div class="float-layer" style={{ width: `${ui.viewport.width}px`, height: `${ui.viewport.height}px` }}>
      <Sidebar ref={sidebar.ref} shown={sidebar.shown()} />
      <Show when={ui.modal}>{(modal) => <Modal modal={modal()} />}</Show>
    </div>
  );
}
