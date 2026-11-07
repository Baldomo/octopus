import { batch, createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";
import { mac } from "./commands";
import { showClientMenu, ui } from "./state";

type Point = { x: number; y: number };
type Opened = { id: string; anchor: HTMLElement; point: Point | null; body: () => JSX.Element };

const [opened, setOpened] = createSignal<Opened | null>(null);

export const isMenuOpen = (id: string) => opened()?.id === id;
export const anyMenuOpen = () => opened() !== null;
export const closePopover = () => setOpened(null);
/** Closes the popover and the client menu, as one menu is open at a time. */
export function closeMenu() {
  closePopover();
  if (ui.client_menu) showClientMenu(false);
}

/** In one batch, so a rename that replaces the menu keeps the keyboard in the sidebar throughout. */
export const closing = (action: () => void) => () =>
  batch(() => {
    closeMenu();
    action();
  });

export function openMenu(id: string, anchor: HTMLElement, body: () => JSX.Element, point: Point | null = null) {
  if (ui.client_menu) showClientMenu(false);
  setOpened(point === null && isMenuOpen(id) ? null : { id, anchor, point, body });
}

export const menuTrigger = (id: string, anchor: () => HTMLElement, body: () => JSX.Element) => ({
  onClick: (event: MouseEvent) => {
    event.stopPropagation();
    openMenu(id, anchor(), body);
  },
  onPointerDown: (event: PointerEvent) => event.stopPropagation(),
  "data-menu-trigger": "",
});

export const onRightClick = (id: string, anchor: () => HTMLElement, body: () => JSX.Element) => (event: MouseEvent) => {
  event.preventDefault();
  event.stopPropagation();
  openMenu(id, anchor(), body, { x: event.clientX, y: event.clientY });
};

function place(menu: HTMLElement, within: HTMLElement, { anchor, point }: Opened) {
  const s = within.getBoundingClientRect();
  const a = anchor.getBoundingClientRect();
  const m = menu.getBoundingClientRect();
  const x = Math.min(point ? point.x - s.left : a.left - s.left, s.width - m.width - 8);
  let y = point ? point.y - s.top : a.bottom - s.top + 4;
  if (y + m.height > s.height - 8) y = (point ? point.y - s.top : a.top - s.top - 4) - m.height;
  menu.style.left = `${within.scrollLeft + Math.max(8, x)}px`;
  menu.style.top = `${within.scrollTop + Math.max(8, y)}px`;
}

const BLUR_SETTLES_MS = 150;

export function dismissible(element: HTMLElement, close: () => void) {
  const outside = (event: PointerEvent) => {
    const target = event.target as Element;
    if (!element.contains(target) && target.closest("[data-menu-trigger]") === null) close();
  };
  const escape = (event: KeyboardEvent) => {
    if (event.key !== "Escape") return;
    event.stopPropagation();
    close();
  };
  document.addEventListener("pointerdown", outside, { capture: true });
  document.addEventListener("keydown", escape, { capture: true });
  // A click outside the float layer lands in another webview, which takes the keyboard from this one. The keyboard
  // also passes through the base layer when one menu closes just before the next opens, so only a lasting loss counts.
  const left = () => setTimeout(() => document.hasFocus() || close(), BLUR_SETTLES_MS);
  window.addEventListener("blur", left);
  onCleanup(() => {
    document.removeEventListener("pointerdown", outside, { capture: true });
    document.removeEventListener("keydown", escape, { capture: true });
    window.removeEventListener("blur", left);
  });
}

function Popover(props: { menu: Opened }) {
  let element!: HTMLDivElement;
  onMount(() => {
    const replace = new ResizeObserver(() => place(element, element.parentElement!, props.menu));
    replace.observe(element);
    onCleanup(() => replace.disconnect());
    dismissible(element, closePopover);
  });
  return (
    <div ref={element} class="menu" role="menu">
      {props.menu.body()}
    </div>
  );
}

export function MenuHost() {
  onCleanup(closePopover);
  return (
    <Show when={opened()} keyed>
      {(menu) => <Popover menu={menu} />}
    </Show>
  );
}

export function Head(props: { title: string; lines: string[]; color?: string }) {
  return (
    <div class="mh">
      <b>
        <Show when={props.color}>{(color) => <i style={{ background: color() }} />}</Show>
        {props.title}
      </b>
      <For each={props.lines}>{(line) => <span class="mono">{line}</span>}</For>
    </div>
  );
}

export const Sep = () => <div class="sep" />;

export function Item(props: {
  children: JSX.Element;
  run: () => void;
  why?: string;
  kbd?: string;
  danger?: boolean;
  check?: boolean;
}) {
  const disabled = () => props.why !== undefined;
  return (
    <button
      class="mi"
      classList={{ danger: props.danger === true && !disabled() }}
      role="menuitem"
      aria-disabled={disabled() ? "true" : undefined}
      onClick={() => disabled() || props.run()}
    >
      <Show when={props.check !== undefined}>
        <span class="ck">{props.check ? "✓" : ""}</span>
      </Show>
      {props.children}
      <Show when={disabled()} fallback={<Show when={props.kbd}>{(kbd) => <kbd>{kbd()}</kbd>}</Show>}>
        <span class="why">{props.why}</span>
      </Show>
    </button>
  );
}

export function Confirm(props: { children: JSX.Element; act: string; run: () => void; cancel: () => void }) {
  return (
    <div class="confirm">
      <span>{props.children}</span>
      <div class="row">
        <button class="btn" onClick={props.cancel}>
          Cancel
        </button>
        <button class="btn danger" onClick={props.run}>
          {props.act}
        </button>
      </div>
    </div>
  );
}

export const copyKey = mac ? "⌘C" : "Ctrl+C";

/** While the menu shows something to copy, the copy shortcut copies it. */
export function copyOnShortcut<T>(shown: () => T | null, copy: (shown: T) => void) {
  const onKey = (event: KeyboardEvent) => {
    const showing = shown();
    if (showing === null || event.key !== "c" || !(mac ? event.metaKey : event.ctrlKey)) return;
    event.preventDefault();
    copy(showing);
  };
  document.addEventListener("keydown", onKey);
  onCleanup(() => document.removeEventListener("keydown", onKey));
}
