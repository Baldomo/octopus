import { createEffect, createMemo, createSignal, on, onCleanup, Show, type JSX } from "solid-js";
import { Dynamic } from "solid-js/web";
import { backend } from "@octopus/shared/backend.ts";
import { Board } from "./Board";
import { desktop, desktopEvents } from "./backend";
import { Browser, forgetBrowsersExcept, openLink } from "./Browser";
import { ClientMenu } from "@octopus/shared/ClientMenu.tsx";
import { Files } from "@octopus/shared/Files.tsx";
import { filesOf, forgetFilesExcept, nameOf, parentOf } from "@octopus/shared/taskFiles.ts";
import { commands, hint } from "@octopus/shared/commands.ts";
import { NewTask } from "@octopus/shared/NewTask.tsx";
import { JoinAccount, PairServer } from "@octopus/shared/PairServer.tsx";
import { Palette } from "@octopus/shared/Palette.tsx";
import { Shortcuts } from "@octopus/shared/Shortcuts.tsx";
import { Sidebar } from "./Sidebar";
import { holdsKeyboard } from "@octopus/shared/SidebarControls.tsx";
import { Toast, toast, toasting } from "@octopus/shared/Toast.tsx";
import { act, model, openView, report, serverView, showModal, showSidebar, ui, uiAct } from "@octopus/shared/state.ts";
import { floatTop } from "./layer";
import { liftOnDrag } from "./lift";
import { StatusBar } from "./StatusBar";
import { TaskSplit } from "./TaskSplit";
import { TerminalStacks } from "@octopus/shared/Terminals.tsx";
import { TitleBar } from "./TitleBar";
import type { Bounds, Modal as ModalName } from "@octopus/shared/generated.ts";
import type { Hovered } from "./generated";

/** This webview's own size, unrounded, which grows only once the app has placed it and its renderer has taken the new size. */
const viewport = visualViewport!;
const [webview, setWebview] = createSignal({ width: viewport.width, height: viewport.height });
viewport.addEventListener("resize", () => setWebview({ width: viewport.width, height: viewport.height }));

/** The app places the layer in whole device pixels, so it may stop short of a surface's far edge by up to one. */
const PLACEMENT_SLACK = 1;

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
  const sink = () => uiAct({ do: "sink", surface: name });
  /** The surface's far corner in this webview's coordinates. */
  const [reach, setReach] = createSignal<{ right: number; bottom: number } | null>(null);
  /** A null `input` takes no pointer. */
  const ref = (element: HTMLElement, input: HTMLElement | null = element) => {
    /** Ticks whenever the surface's content changes its size, as a menu growing a QR code does. */
    const [resized, setResized] = createSignal(0);
    const observer = new ResizeObserver(() => setResized((ticks) => ticks + 1));
    observer.observe(element);
    onCleanup(() => observer.disconnect());
    createEffect(
      on(
        () => [out(), modal(), ui.viewport.width, ui.viewport.height, resized()] as const,
        ([isOut, isModal]) => {
          const rect = box(element);
          setReach({ right: rect.x + rect.width, bottom: rect.y + rect.height - floatTop() });
          if (isOut)
            uiAct({
              do: "float",
              surface: name,
              at: { input: input === null ? NO_INPUT : box(input), drawn: drawnBox(element) },
              modal: isModal,
            });
          else if (element.getAnimations().length === 0) sink();
        },
      ),
    );
    element.addEventListener("transitionend", (event) => {
      if (event.target === element && !out()) sink();
    });
    onCleanup(sink);
  };
  const held = () => {
    const corner = reach();
    return (
      corner !== null &&
      corner.right < webview().width + PLACEMENT_SLACK &&
      corner.bottom < webview().height + PLACEMENT_SLACK
    );
  };
  const shown = createMemo<boolean>((was) => out() && (was || held()), false);
  return { ref, shown };
}

/** The base layer: docked UI takes the window's layout space, and each browser page sits in the rect it leaves. */
export function Docked() {
  createEffect(on(() => [...ui.recent], forgetFilesExcept));
  createEffect(() => forgetBrowsersExcept([...ui.recent]));
  /** The open task is focused whenever it is in the Task view with its server reachable, so a restart refocuses it. */
  const shownInTaskView = createMemo(() => {
    const shown = ui.prefs.view === "task" ? openView() : undefined;
    return shown !== undefined && serverView(shown.at.server)?.reachable ? ui.prefs.open : null;
  });
  createEffect(
    on(shownInTaskView, (key) => {
      if (key !== null) act(backend.taskFocus(openView()!.at));
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
      <TaskSplit
        shown={ui.prefs.view === "task"}
        surface={
          <div class="pane">
            <div class="tabs switch">
              <button class="tab" classList={{ on: ui.prefs.surface === "files" }} onClick={commands.files.run}>
                Files<kbd>{hint(commands.files)}</kbd>
              </button>
              <button class="tab" classList={{ on: ui.prefs.surface === "browser" }} onClick={commands.browser.run}>
                Browser<kbd>{hint(commands.browser)}</kbd>
              </button>
            </div>
            <Show when={openView()} fallback={<div class="empty">No task is open.</div>}>
              <div class="surface" style={{ display: ui.prefs.surface === "files" ? "" : "none" }}>
                <Files
                  visible={ui.prefs.view === "task" && ui.prefs.surface === "files"}
                  drag={liftOnDrag}
                  outside={(task, path, how) => desktop.fileOutside(task.at, path, how)}
                />
              </div>
              <div class="surface" style={{ display: ui.prefs.surface === "browser" ? "" : "none" }}>
                <Browser visible={ui.prefs.view === "task" && ui.prefs.surface === "browser"} />
              </div>
            </Show>
          </div>
        }
        terminals={<TerminalStacks openLink={openLink} />}
      />
      <div class="tasks-view" style={{ display: ui.prefs.view === "tasks" ? "" : "none" }}>
        <Board />
      </div>
      <StatusBar />
      {/* A pointer back from a page reaches the edge with no pointerenter, as this webview never saw it leave. */}
      <div class="edge" classList={{ handle: !ui.sidebar }} onPointerMove={() => ui.sidebar || showSidebar(true)} />
    </div>
  );
}

/** While the sidebar is hidden, its toasts show at the foot of the window. */
function FloatingToast() {
  const toast = floating({ name: "toast", modal: () => false, out: toasting });
  return <Toast ref={(element) => toast.ref(element, null)} />;
}

/** The link under the pointer shows at the page's bottom-left corner, over the page, at most half its width. */
function PageStatus() {
  const [hovered, setHovered] = createSignal<Hovered | null>(null);
  const status = floating({ name: "page_status", modal: () => false, out: () => hovered() !== null });
  const unlisten = desktopEvents.browserHovered(setHovered);
  onCleanup(() => unlisten.then((stop) => stop(), report));
  const page = () => hovered()?.page;
  return (
    <div
      ref={(element) => status.ref(element, null)}
      class="page-status"
      classList={{ on: status.shown() }}
      style={{
        left: `${page()?.x ?? 0}px`,
        top: `${(page()?.y ?? 0) + (page()?.height ?? 0)}px`,
        width: `${(page()?.width ?? 0) / 2}px`,
      }}
    >
      <span>{hovered()?.link}</span>
    </div>
  );
}

function FloatingClientMenu() {
  const menu = floating({ name: "client_menu", modal: () => true, out: () => true });
  return <ClientMenu floats={menu.ref} shown={menu.shown()} />;
}

const MODALS: Record<ModalName, () => JSX.Element> = {
  palette: Palette,
  new_task: NewTask,
  pair_server: PairServer,
  join_account: JoinAccount,
  shortcuts: Shortcuts,
};

function Modal(props: { modal: ModalName }) {
  const scrim = floating({ name: "modal", modal: () => true, out: () => true });
  return (
    <>
      <div class="scrim" ref={scrim.ref} onClick={() => showModal(null)} />
      <Show when={scrim.shown()}>
        <Dynamic component={MODALS[props.modal]} />
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
  const unlisten = desktopEvents.browserDownloaded(({ path, success }) => {
    const [folder, name] = [parentOf(path), nameOf(path)];
    if (success)
      toast(
        <>
          Downloaded {name} to {folder}
        </>,
      );
    else report(`The download of ${name} failed`);
  });
  onCleanup(() => unlisten.then((stop) => stop(), report));
  return (
    <div class="float-layer" style={{ width: `${ui.viewport.width}px`, height: `${ui.viewport.height}px` }}>
      <PageStatus />
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
