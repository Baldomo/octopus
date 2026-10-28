import { createEffect, createMemo, createSignal, Match, on, onCleanup, Show, Switch } from "solid-js";
import { backend } from "@octopus/shared/backend.ts";
import { Board } from "./Board";
import { Browser, forgetBrowsersExcept } from "./Browser";
import { ClientMenu } from "@octopus/shared/ClientMenu.tsx";
import { Files } from "@octopus/shared/Files.tsx";
import { filesOf, forgetFilesExcept } from "@octopus/shared/taskFiles.ts";
import { commands, hint } from "@octopus/shared/commands.ts";
import { NewTask } from "@octopus/shared/NewTask.tsx";
import { PairServer } from "@octopus/shared/PairServer.tsx";
import { Palette } from "@octopus/shared/Palette.tsx";
import { Shortcuts } from "@octopus/shared/Shortcuts.tsx";
import { Sidebar } from "./Sidebar";
import { holdsKeyboard, Toast, toasting } from "@octopus/shared/SidebarControls.tsx";
import { act, model, openView, serverView, showModal, showSidebar, ui } from "@octopus/shared/state.ts";
import { floatSurface, floatTop, sinkSurface } from "./layer";
import { StatusBar } from "./StatusBar";
import { TerminalStacks } from "@octopus/shared/Terminals.tsx";
import { TitleBar } from "./TitleBar";
import type { Bounds, Modal as ModalName } from "@octopus/shared/generated.ts";

/** This webview's own size, which grows only once the app has placed it and its renderer has taken the new size. */
const [webview, setWebview] = createSignal({ width: innerWidth, height: innerHeight });
addEventListener("resize", () => setWebview({ width: innerWidth, height: innerHeight }));

/** The element's rect in window coordinates. */
const box = (element: HTMLElement): Bounds => ({
  x: element.offsetLeft,
  y: element.offsetTop,
  width: element.offsetWidth,
  height: element.offsetHeight,
});

/** The element's box grown by how far its outer box shadows reach past each edge, its far edges cut to the window. */
function drawnBox(element: HTMLElement): Bounds {
  const rect = box(element);
  const shadows = getComputedStyle(element)
    .boxShadow.replace(/[\w-]+\([^)]*\)/g, "")
    .split(",");
  const reach = { left: 0, right: 0, top: 0, bottom: 0 };
  for (const shadow of shadows.filter((shadow) => !shadow.includes("inset"))) {
    const [x = 0, y = 0, blur = 0, spread = 0] = (shadow.match(/-?[\d.]+px/g) ?? []).map(parseFloat);
    reach.left = Math.max(reach.left, blur + spread - x);
    reach.right = Math.max(reach.right, blur + spread + x);
    reach.top = Math.max(reach.top, blur + spread - y);
    reach.bottom = Math.max(reach.bottom, blur + spread + y);
  }
  const [left, top] = [rect.x - reach.left, rect.y - reach.top];
  const right = Math.min(rect.x + rect.width + reach.right, ui.viewport.width);
  const bottom = Math.min(rect.y + rect.height + reach.bottom, ui.viewport.height);
  return { x: left, y: top, width: right - left, height: bottom - top };
}

const NO_INPUT: Bounds = { x: 0, y: 0, width: 0, height: 0 };

/**
 * A surface of the float layer: while it is `out`, and until its exit transition ends, the app covers what `element`
 * draws with the float layer, which takes the pointer in `input`, and a surface holds the keyboard while it is `modal`.
 * It is `shown` from the moment the webview holds it, so it never draws where the window cannot show it yet.
 */
function floating(surface: { name: string; modal: () => boolean; out: () => boolean }) {
  const { name, modal, out } = surface;
  /** The surface's far corner in this webview's coordinates. */
  const [reach, setReach] = createSignal<{ right: number; bottom: number } | null>(null);
  /** A null `input` takes no pointer. */
  const ref = (element: HTMLElement, input: HTMLElement | null = element) => {
    createEffect(
      on(
        () => [out(), modal(), ui.viewport.width, ui.viewport.height] as const,
        ([isOut, isModal]) => {
          const rect = box(element);
          setReach({ right: rect.x + rect.width, bottom: rect.y + rect.height - floatTop() });
          if (isOut)
            floatSurface(name, { input: input === null ? NO_INPUT : box(input), drawn: drawnBox(element) }, isModal);
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
              <Files visible={ui.prefs.view === "task" && ui.prefs.surface === "files"} />
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
      <div class="edge" classList={{ handle: !ui.sidebar }} onPointerEnter={() => showSidebar(true)} />
    </div>
  );
}

/** While the sidebar is hidden, its toasts show at the foot of the window. */
function FloatingToast() {
  const toast = floating({ name: "toast", modal: () => false, out: toasting });
  return <Toast ref={(element) => toast.ref(element, null)} />;
}

function FloatingClientMenu() {
  const menu = floating({ name: "client_menu", modal: () => true, out: () => true });
  return <ClientMenu floats={menu.ref} shown={menu.shown()} />;
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
          <Match when={props.modal === "shortcuts"}>
            <Shortcuts />
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
      <Sidebar floats={sidebar.ref} shown={sidebar.shown()} />
      <Show when={!ui.sidebar}>
        <FloatingToast />
      </Show>
      <Show when={ui.client_menu}>
        <FloatingClientMenu />
      </Show>
      <Show when={ui.modal}>{(modal) => <Modal modal={modal()} />}</Show>
    </div>
  );
}
