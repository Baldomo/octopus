import { createEffect, createSignal, For, on, onCleanup, onMount, Show } from "solid-js";
import { createStore, produce } from "solid-js/store";
import { backend, events } from "./backend";
import { NEW_BROWSER_PANEL } from "./generated";
import type { BrowserId, BrowserPanel, Go, OnScreen, TaskRef } from "./generated";
import { act, openView, report, ui } from "./state";

type Target = BrowserId | typeof NEW_BROWSER_PANEL;

/** The panel each task's URL bar drives on this client; `new` makes the next page open a panel of its own. */
const [selected, setSelected] = createStore<Record<string, Target>>({});
/** Per task key, the panels whose webview exists on this client, each with the port its page tunnels to. */
const [rendered, setRendered] = createStore<Record<string, { at: TaskRef; tunnels: Record<BrowserId, number | null> }>>(
  {},
);
/** Per task key, each rendered panel's page as it loaded, which moves off its server `url` as the user navigates. */
const [loaded, setLoaded] = createStore<Record<string, Record<BrowserId, string>>>({});
events
  .browserNavigated((navigated) =>
    setLoaded(produce((pages) => ((pages[navigated.task] ??= {})[navigated.panel] = navigated.url))),
  )
  .catch(report);

/** Native webview calls run in the order the UI makes them, so a page exists before it is placed or closed. */
let queue: Promise<unknown> = Promise.resolve();
const serially = <T,>(call: () => Promise<T>): Promise<T> => {
  const next = queue.then(call);
  queue = next.catch(() => undefined);
  return next;
};

export function forgetBrowsersExcept(keep: string[]) {
  for (const [task, { at, tunnels }] of Object.entries(rendered).filter(([task]) => !keep.includes(task))) {
    setRendered(produce((tasks) => delete tasks[task]));
    setLoaded(produce((pages) => delete pages[task]));
    for (const panel of Object.keys(tunnels)) act(serially(() => backend.browserClose(at, panel)));
  }
}

const withScheme = (typed: string) => (/^[a-z][a-z0-9+.-]*:\/\//i.test(typed) ? typed : `http://${typed}`);

const hostOf = (url: string) => {
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
};

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
  const panels = (): BrowserPanel[] => openView()?.task.browsers ?? [];
  /** The task's chosen panel while it exists, else its newest one. */
  const target = (): Target => {
    const task = openView();
    const chosen = task === undefined ? undefined : selected[task.key];
    if (chosen === NEW_BROWSER_PANEL || panels().some((panel) => panel.panel === chosen))
      return chosen ?? NEW_BROWSER_PANEL;
    return panels().at(-1)?.panel ?? NEW_BROWSER_PANEL;
  };
  const current = () => panels().find((panel) => panel.panel === target());
  const tunnelOf = (task: string, panel: BrowserId) => rendered[task]?.tunnels[panel];
  const shownUrl = () => {
    const task = openView();
    const panel = current();
    return task === undefined || panel === undefined ? undefined : (loaded[task.key]?.[panel.panel] ?? panel.url);
  };
  const tunnel = () => {
    const task = openView();
    const panel = current();
    return task === undefined || panel === undefined ? null : (tunnelOf(task.key, panel.panel) ?? null);
  };

  /** Each time the chosen panel's server `url` moves, this client loads it. */
  createEffect(
    on(
      () => [openView()?.at, openView()?.key, current()?.panel, current()?.url] as const,
      ([at, task, panel, url]) => {
        if (at === undefined || task === undefined || panel === undefined || url === undefined) return;
        setLoaded(produce((pages) => delete pages[task]?.[panel]));
        serially(() => backend.browserLoad(at, panel, url)).then((port) => {
          if (!ui.recent.includes(task)) return act(serially(() => backend.browserClose(at, panel)));
          setRendered(produce((tasks) => ((tasks[task] ??= { at, tunnels: {} }).tunnels[panel] = port)));
        }, report);
      },
    ),
  );

  const onScreen = (): Omit<OnScreen, "bounds"> | null => {
    const task = openView();
    const panel = current();
    if (!props.visible || task === undefined || panel === undefined) return null;
    return tunnelOf(task.key, panel.panel) === undefined ? null : { at: task.at, panel: panel.panel };
  };

  /**
   * A page arrives as this layer paints the frame it is laid out in, and leaves only once this layer has painted what
   * it uncovers: two processes paint the window, so a page may outstay its frame, but never leaves a hole.
   */
  let latest = 0;
  const showPage = (shown: Omit<OnScreen, "bounds"> | null) => {
    const placing = ++latest;
    const send = () => {
      if (placing !== latest) return;
      const { left, top, width, height } = page.getBoundingClientRect();
      const bounds = { x: left, y: top, width, height };
      act(serially(() => backend.browserShowPage(shown === null ? null : { ...shown, bounds })));
    };
    requestAnimationFrame(shown === null ? () => setTimeout(send) : send);
  };
  createEffect(() => showPage(onScreen()));
  onMount(() => {
    const observer = new ResizeObserver(() => showPage(onScreen()));
    observer.observe(page);
    onCleanup(() => observer.disconnect());
  });

  const go = (typed: string) => {
    const task = openView();
    if (task === undefined || typed.trim() === "") return;
    backend
      .browserOpen(task.at, target(), withScheme(typed.trim()))
      .then((panel) => setSelected(task.key, panel), report);
  };

  const navigate = (step: Go) => {
    const shown = onScreen();
    if (shown !== null) act(serially(() => backend.browserGo(shown.at, shown.panel, step)));
  };

  const choose = (panel: Target) => {
    const task = openView();
    if (task !== undefined) setSelected(task.key, panel);
  };

  return (
    <div class="browser">
      <Show when={panels().length > 1}>
        <div class="tabs">
          <For each={panels()}>
            {(panel) => (
              <button class="tab" classList={{ on: target() === panel.panel }} onClick={() => choose(panel.panel)}>
                {hostOf(panel.url)}
              </button>
            )}
          </For>
        </div>
      </Show>
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
          <Show when={editing() || shownUrl() === undefined} fallback={<Address url={shownUrl()!} />}>
            <input
              ref={(input) => queueMicrotask(() => editing() && input.focus())}
              value={shownUrl() ?? ""}
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
          <Show when={tunnel()}>
            {(port) => (
              <>
                <span class="spacer" />
                <span class="tag live">
                  ⇄ {openView()?.at.server}:{port()}
                </span>
              </>
            )}
          </Show>
        </div>
        <span class="navb" title="new browser panel" onClick={() => choose(NEW_BROWSER_PANEL)}>
          +
        </span>
      </div>
      <div class="page" ref={page}>
        <Show when={current() === undefined}>
          <div class="empty">Type a URL to open a page in a new browser panel of this task.</div>
        </Show>
      </div>
    </div>
  );
}
