import { createEffect, createSignal, onCleanup, onMount, Show } from "solid-js";
import { createStore, produce } from "solid-js/store";
import { backend, events } from "./backend";
import type { Bounds, Go } from "./generated";
import { act, open, overlay, recent, report, sidebar } from "./state";
import { px } from "./tokens";

/** Browser panels live on this client, one per task, keyed by the task. */
const [urls, setUrls] = createStore<Record<string, string>>({});
/** Panels that own a native webview; the rest have no page yet. */
const [created, setCreated] = createStore<Record<string, boolean>>({});
events.browserNavigated((navigated) => setUrls(navigated.panel, navigated.url)).catch(report);

/** Native webview calls run one at a time: a hide racing past an earlier show would leave a page on screen. */
let queue: Promise<unknown> = Promise.resolve();
const serially = <T,>(call: () => Promise<T>): Promise<T> => {
  const next = queue.then(call);
  queue = next.catch(() => undefined);
  return next;
};

export function forgetBrowsersExcept(keep: string[]) {
  for (const panel of Object.keys(created).filter((panel) => !keep.includes(panel))) {
    setCreated(produce((panels) => delete panels[panel]));
    setUrls(produce((pages) => delete pages[panel]));
    act(serially(() => backend.browserClose(panel)));
  }
}

const withScheme = (typed: string) => (/^[a-z][a-z0-9+.-]*:\/\//i.test(typed) ? typed : `http://${typed}`);

function Address(props: { url: string }) {
  const parsed = () => {
    try {
      const url = new URL(props.url);
      return { host: url.host, rest: `${url.pathname === "/" ? "" : url.pathname}${url.search}${url.hash}` };
    } catch {
      return { host: props.url, rest: "" };
    }
  };
  return (
    <>
      <b>{parsed().host}</b>
      {parsed().rest}
    </>
  );
}

export function Browser(props: { visible: boolean }) {
  let page!: HTMLDivElement;
  const [editing, setEditing] = createSignal(false);
  const url = () => {
    const panel = open();
    return panel === null ? undefined : urls[panel];
  };

  /** The page's rect, clear of the sidebar: a native webview draws above every element of the UI. */
  const bounds = (sidebarShown: boolean): Bounds => {
    const rect = page.getBoundingClientRect();
    const clear = sidebarShown ? px("--sidebar-w") + 2 * px("--s2") : 0;
    const x = Math.max(rect.left, clear);
    return { x, y: rect.top, width: Math.max(0, rect.right - x), height: rect.height };
  };
  const onScreen = () => {
    const panel = open();
    return props.visible && overlay() === null && panel !== null && created[panel] ? panel : null;
  };

  let placed: string | null = null;
  const place = () => {
    const panel = onScreen();
    const sidebarShown = sidebar();
    const stale = placed;
    if (stale !== null && stale !== panel) act(serially(() => backend.browserPlace(stale, null)));
    if (panel !== null) {
      const shown = bounds(sidebarShown);
      act(serially(() => backend.browserPlace(panel, shown)));
    }
    placed = panel;
  };
  createEffect(place);
  onMount(() => {
    const observer = new ResizeObserver(place);
    observer.observe(page);
    onCleanup(() => observer.disconnect());
  });

  const go = (typed: string) => {
    const panel = open();
    if (panel === null || typed.trim() === "") return;
    const target = withScheme(typed.trim());
    serially(() => backend.browserOpen(panel, target)).then(() => {
      if (!recent().includes(panel)) return act(serially(() => backend.browserClose(panel)));
      setUrls(panel, target);
      setCreated(panel, true);
    }, report);
  };

  const navigate = (step: Go) => {
    const panel = open();
    if (panel !== null && created[panel]) act(serially(() => backend.browserGo(panel, step)));
  };

  return (
    <div class="browser">
      <div class="urlbar">
        <span class="navb" onClick={() => navigate("back")}>
          ‹
        </span>
        <span class="navb" onClick={() => navigate("forward")}>
          ›
        </span>
        <span class="navb" onClick={() => navigate("reload")}>
          ↻
        </span>
        <div class="url" onClick={() => setEditing(true)}>
          <Show when={editing() || url() === undefined} fallback={<Address url={url()!} />}>
            <input
              ref={(input) => queueMicrotask(() => editing() && input.focus())}
              value={url() ?? ""}
              placeholder="localhost:5173"
              spellcheck={false}
              onBlur={() => setEditing(false)}
              onKeyDown={(event) => {
                if (event.key !== "Enter") return;
                go(event.currentTarget.value);
                setEditing(false);
              }}
            />
          </Show>
        </div>
      </div>
      <div class="page" ref={page}>
        <Show when={url() === undefined}>
          <div class="empty">Type a URL to open a page in this task's browser panel.</div>
        </Show>
      </div>
    </div>
  );
}
