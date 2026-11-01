import { createEffect, createMemo, createSignal, on, onCleanup, onMount, Show, untrack } from "solid-js";
import { createStore, produce } from "solid-js/store";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { BACKGROUND_BROWSER_PANEL, NEW_BROWSER_PANEL } from "@octopus/shared/generated.ts";
import { provideBrowser } from "@octopus/shared/commands.ts";
import { FolderTabs, type FolderTab } from "@octopus/shared/FolderTabs.tsx";
import type { BrowserId, BrowserPanel, TaskRef, TaskView } from "@octopus/shared/generated.ts";
import { act, model, openView, report, serverLabel, showSurface, ui } from "@octopus/shared/state.ts";
import type { OpenLink } from "@octopus/shared/Terminals.tsx";
import { desktop, desktopEvents } from "./backend";
import { liftOnDrag } from "./lift";
import { cycled, heirOf, openedAt } from "./tabOrder";
import type { Go, OnScreen } from "./generated";

type Target = BrowserId | typeof NEW_BROWSER_PANEL;

/** A rendered panel's page on this client, which moves off its server `url` as the user navigates. */
type Page = {
  url: string;
  loading: boolean;
  back: boolean;
  forward: boolean;
  tunnel: number | null;
  failed: { url: string; error: string } | null;
};

/** The panel each task's tabs select on this client; `new` makes the next page open a panel of its own. */
const [selected, setSelected] = createStore<Record<string, Target>>({});
/** Per task key, the task and the panels whose webview exists on this client. */
const [rendered, setRendered] = createStore<Record<string, { at: TaskRef; panels: BrowserId[] }>>({});
const [pages, setPages] = createStore<Record<string, Record<BrowserId, Page>>>({});
/** Per task key, each panel's `load` this client last saw; a `load` that moves selects its panel. */
const seenLoads = new Map<string, Map<BrowserId, number>>();
/** Per task key, the order of its panels this client last saw. */
const seenOrders = new Map<string, BrowserId[]>();
/** Per task key, the panel whose page opened each panel a page opened on this client. */
const openers = new Map<string, Map<BrowserId, BrowserId>>();
/** Per task key, the tabs this client closed, the last closed last, each with its page and its index. */
const closed = new Map<string, { url: string; at: number }[]>();
const entryOf = <T,>(all: Map<string, T>, task: string, fresh: () => T) => {
  if (!all.has(task)) all.set(task, fresh());
  return all.get(task)!;
};
const [editing, setEditing] = createSignal(false);
/** While finding, the address field takes the text to find in the page, which it keeps for the next find. */
const [finding, setFinding] = createSignal(false);
const [sought, setSought] = createSignal("");
let address: HTMLInputElement | undefined;

const blank: Page = { url: "", loading: false, back: false, forward: false, tunnel: null, failed: null };
const page = (task: string, panel: BrowserId): Page | undefined => pages[task]?.[panel];
const setPage = (task: string, panel: BrowserId, change: Partial<Page>) =>
  setPages(produce((all) => ((all[task] ??= {})[panel] = { ...blank, ...all[task]?.[panel], ...change })));

desktopEvents
  .browserPaged(({ task, panel, url, loading, back, forward, tunnel }) =>
    setPage(task, panel, { url, loading, back, forward, tunnel, ...(loading ? { failed: null } : {}) }),
  )
  .catch(report);
desktopEvents
  .browserFailed(({ task, panel, url, error }) => setPage(task, panel, { failed: { url, error }, loading: false }))
  .catch(report);

/** Native webview calls run in the order the UI makes them, so a page exists before it is placed or closed. */
let queue: Promise<unknown> = Promise.resolve();
const serially = <T,>(call: () => Promise<T>): Promise<T> => {
  const next = queue.then(call);
  queue = next.catch(() => undefined);
  return next;
};

function forget(task: string, at: TaskRef, panel: BrowserId) {
  setRendered(
    produce((tasks) => {
      const entry = tasks[task];
      if (entry !== undefined) entry.panels = entry.panels.filter((known) => known !== panel);
    }),
  );
  setPages(produce((all) => delete all[task]?.[panel]));
  act(serially(() => desktop.browserForget(at, panel)));
}

/** Closes the webviews of the tasks no longer among `keep`, and of every panel its server closed. */
export function forgetBrowsersExcept(keep: string[]) {
  for (const [task, { at, panels }] of Object.entries(rendered)) {
    const open = keep.includes(task) ? (model.tasks[task]?.task.browsers ?? []) : [];
    for (const panel of panels.filter((panel) => !open.some((known) => known.panel === panel))) {
      forget(task, at, panel);
    }
  }
}

/** Loads the panel's page on this client unless it already did, and keeps its webview while the task is recent. */
function loadPanel(task: TaskView, panel: BrowserId, url: string, load: number) {
  const at = task.at;
  return serially(() => desktop.browserLoad(at, panel, url, load)).then(() => {
    if (!ui.recent.includes(task.key)) return act(serially(() => desktop.browserForget(at, panel)));
    setRendered(
      produce((tasks) => {
        const entry = (tasks[task.key] ??= { at, panels: [] });
        if (!entry.panels.includes(panel)) entry.panels.push(panel);
      }),
    );
  }, report);
}

/**
 * Opens `url` in a new tab after the `opener`'s tab and the tabs it opened before. A tab opened behind loads on this
 * client at once, and no client selects it.
 */
async function openFrom(task: TaskView, opener: BrowserId, url: string, select: boolean) {
  const panel = await desktop.browserOpen(task.at, select ? NEW_BROWSER_PANEL : BACKGROUND_BROWSER_PANEL, url);
  const opened = entryOf(openers, task.key, () => new Map());
  opened.set(panel, opener);
  if (!select) loadPanel(task, panel, url, 0);
  const others = (model.tasks[task.key]?.task.browsers ?? []).map((known) => known.panel).filter((id) => id !== panel);
  await desktop.browserMove(
    task.at,
    panel,
    openedAt(others, opener, (tab) => opened.get(tab)),
  );
}

/** Opens the tab this client closed last in the task again, at its index, and selects it. */
async function reopen(task: TaskView) {
  const last = closed.get(task.key)?.pop();
  if (last === undefined) return;
  const panel = await desktop.browserOpen(task.at, NEW_BROWSER_PANEL, last.url);
  await desktop.browserMove(task.at, panel, Math.min(last.at, task.task.browsers.length));
}

/** The task's chosen panel while it exists, else its last one. */
function targetOf(task: TaskView): Target {
  const chosen = selected[task.key];
  const panels = task.task.browsers;
  if (chosen === NEW_BROWSER_PANEL || panels.some((panel) => panel.panel === chosen))
    return chosen ?? NEW_BROWSER_PANEL;
  return panels.at(-1)?.panel ?? NEW_BROWSER_PANEL;
}

const currentOf = (task: TaskView) => task.task.browsers.find((panel) => panel.panel === targetOf(task));

/** Shows `url` in the task's target panel, which stays the target. */
const openPage = (task: TaskView, url: string) =>
  desktop.browserOpen(task.at, targetOf(task), url).then((panel) => setSelected(task.key, panel), report);

/** A terminal's link opens in the task's browser panel, the left side switching to Browser, or in the system's browser. */
export const openLink: OpenLink = (task, url, target) => {
  if (target === "system") act(desktop.openOutside(url));
  else {
    showSurface("browser");
    openPage(task, url);
  }
};

/** What the user typed as a page: a URL with its scheme, a file's absolute path, or a web address. */
const typedPage = (typed: string) => {
  if (/^[a-z][a-z0-9+.-]*:\/\//i.test(typed)) return typed;
  if (typed.startsWith("/")) return new URL(`file://${typed}`).href;
  return `http://${typed}`;
};

/** What a page is called before its title arrives: a file's name, else its host. */
const nameOf = (url: string) => {
  try {
    const parsed = new URL(url);
    return parsed.protocol === "file:" ? decodeURIComponent(parsed.pathname.split("/").at(-1) ?? "") : parsed.host;
  } catch {
    return url;
  }
};

function Address(props: { url: string }) {
  const parts = () => {
    try {
      const url = new URL(props.url);
      if (url.protocol === "file:") {
        const path = decodeURIComponent(url.pathname);
        const cut = path.lastIndexOf("/") + 1;
        return { lead: path.slice(0, cut), main: path.slice(cut), rest: `${url.search}${url.hash}` };
      }
      const rest = `${url.pathname === "/" ? "" : url.pathname}${url.search}${url.hash}`;
      return { lead: "", main: url.host, rest };
    } catch {
      return { lead: "", main: props.url, rest: "" };
    }
  };
  return (
    <span class="name">
      {parts().lead}
      <b>{parts().main}</b>
      {parts().rest}
    </span>
  );
}

export function Browser(props: { visible: boolean }) {
  let surface!: HTMLDivElement;
  const panels = (): BrowserPanel[] => openView()?.task.browsers ?? [];
  const target = (): Target => {
    const task = openView();
    return task === undefined ? NEW_BROWSER_PANEL : targetOf(task);
  };
  const current = () => {
    const task = openView();
    return task === undefined ? undefined : currentOf(task);
  };
  const shown = () => {
    const task = openView();
    const panel = current();
    return task === undefined || panel === undefined ? undefined : page(task.key, panel.panel);
  };
  const shownUrl = () => shown()?.failed?.url || shown()?.url || current()?.url;

  /**
   * A panel whose `load` moved since this client last saw the task selects itself, a new panel counting from 0;
   * otherwise the selection holds on to its panel, so panels moving around it leave it where it is, and passes to its
   * heir when it closes.
   */
  createEffect(() => {
    const task = openView();
    if (task === undefined) return;
    const seen = seenLoads.get(task.key);
    const loads = new Map(task.task.browsers.map((panel) => [panel.panel, panel.load]));
    const moved =
      seen === undefined
        ? undefined
        : task.task.browsers.findLast((panel) => (seen.get(panel.panel) ?? 0) !== panel.load);
    seenLoads.set(task.key, loads);
    const order = task.task.browsers.map((panel) => panel.panel);
    const before = seenOrders.get(task.key) ?? order;
    seenOrders.set(task.key, order);
    const chosen = untrack(() => selected[task.key]);
    const gone = chosen !== undefined && chosen !== NEW_BROWSER_PANEL && !order.includes(chosen);
    const heir = gone ? (heirOf(before, order, chosen) ?? NEW_BROWSER_PANEL) : undefined;
    setSelected(task.key, moved?.panel ?? heir ?? untrack(() => targetOf(task)));
  });

  const unlisten = desktopEvents.browserOpened(({ task, panel, url, select }) => {
    const view = model.tasks[task];
    if (view !== undefined) act(openFrom(view, panel, url, select));
  });
  onCleanup(() => unlisten.then((stop) => stop(), report));

  /** Each time the chosen panel's `load` moves, this client loads its `url`. */
  const loading = createMemo(() => {
    const task = openView();
    const panel = current();
    return task === undefined || panel === undefined ? undefined : `${task.key}\n${panel.panel}\n${panel.load}`;
  });
  createEffect(
    on(loading, (key) => {
      if (key === undefined) return;
      const [task, panel] = untrack(() => [openView()!, current()!]);
      loadPanel(task, panel.panel, panel.url, panel.load);
    }),
  );

  const onScreen = (): Omit<OnScreen, "bounds"> | null => {
    const task = openView();
    const panel = current();
    if (!props.visible || task === undefined || panel === undefined || shown()?.failed) return null;
    return rendered[task.key]?.panels.includes(panel.panel) ? { at: task.at, panel: panel.panel } : null;
  };

  /**
   * A page arrives as this layer paints the frame it is laid out in, and leaves only once this layer has painted what
   * it uncovers: two processes paint the window, so a page may outstay its frame, but never leaves a hole.
   */
  let latest = 0;
  const showPage = (onscreen: Omit<OnScreen, "bounds"> | null) => {
    const placing = ++latest;
    const send = () => {
      if (placing !== latest) return;
      const { left, top, width, height } = surface.getBoundingClientRect();
      const bounds = { x: left, y: top, width, height };
      act(serially(() => desktop.browserShowPage(onscreen === null ? null : { ...onscreen, bounds })));
    };
    requestAnimationFrame(onscreen === null ? () => setTimeout(send) : send);
  };
  createEffect(() => showPage(onScreen()));
  onMount(() => {
    const observer = new ResizeObserver(() => showPage(onScreen()));
    observer.observe(surface);
    onCleanup(() => observer.disconnect());
  });

  const open = (typed: string) => {
    const task = openView();
    if (task !== undefined && typed.trim() !== "") openPage(task, typedPage(typed.trim()));
  };

  const navigate = (step: Go) => {
    const task = openView();
    const panel = current();
    const failed = shown()?.failed;
    if (task === undefined || panel === undefined) return;
    if (step.startsWith("reload") && failed) openPage(task, failed.url);
    else act(serially(() => desktop.browserGo(task.at, panel.panel, step)));
  };

  const tabs = (): FolderTab[] =>
    panels().map((panel) => ({
      key: panel.panel,
      name: panel.title || nameOf(panel.url),
      where: undefined,
      title: panel.url,
      loading: page(openView()?.key ?? "", panel.panel)?.loading === true,
      dirty: false,
    }));

  const close = (panel: BrowserId) => {
    const task = openView();
    if (task === undefined) return;
    const at = panels().findIndex((known) => known.panel === panel);
    const url = page(task.key, panel)?.url || panels()[at]?.url;
    if (url !== undefined) entryOf(closed, task.key, () => []).push({ url, at });
    act(desktop.browserClose(task.at, panel));
  };

  const move = async (panel: BrowserId, to: number) => {
    const task = openView();
    if (task !== undefined) await desktop.browserMove(task.at, panel, to);
  };

  const choose = (panel: Target) => {
    const task = openView();
    if (task !== undefined) setSelected(task.key, panel);
    if (panel === NEW_BROWSER_PANEL) locate();
  };

  const locate = () => {
    setEditing(true);
    act(getCurrentWebview().setFocus());
    address?.focus();
    address?.select();
  };

  const find = () => {
    setFinding(true);
    act(getCurrentWebview().setFocus());
  };

  const findInPage = (text: string, backwards: boolean) => {
    const task = openView();
    const panel = current();
    setSought(text);
    if (task !== undefined && panel !== undefined && text !== "")
      act(serially(() => desktop.browserFind(task.at, panel.panel, text, backwards)));
  };

  provideBrowser({
    shows: () => props.visible && current() !== undefined,
    showsTabs: () => props.visible && panels().length > 0,
    reopens: () => (closed.get(openView()?.key ?? "")?.length ?? 0) > 0,
    locate,
    find,
    go: navigate,
    close: () => {
      const panel = current();
      if (panel !== undefined) close(panel.panel);
    },
    add: () => choose(NEW_BROWSER_PANEL),
    reopen: () => {
      const task = openView();
      if (task !== undefined) act(reopen(task));
    },
    cycle: (step) => {
      const from = target();
      const next = cycled(
        panels().map((panel) => panel.panel),
        from === NEW_BROWSER_PANEL ? undefined : from,
        step,
      );
      if (next !== undefined) choose(next);
    },
  });

  return (
    <div class="browser">
      <FolderTabs
        tabs={tabs()}
        selected={target()}
        visible={props.visible}
        drag={liftOnDrag}
        choose={choose}
        close={close}
        move={move}
        add={{
          title: "New browser panel",
          selected: target() === NEW_BROWSER_PANEL,
          run: () => choose(NEW_BROWSER_PANEL),
        }}
      />
      <div class="urlbar">
        <button class="navb" disabled={!shown()?.back} onClick={() => navigate("back")}>
          ‹
        </button>
        <button class="navb" disabled={!shown()?.forward} onClick={() => navigate("forward")}>
          ›
        </button>
        <button class="navb" disabled={current() === undefined} onClick={() => navigate("reload")}>
          ↻
        </button>
        <Show
          when={!finding()}
          fallback={
            <div class="url">
              <input
                ref={(input) => queueMicrotask(() => (input.focus(), input.select()))}
                value={untrack(sought)}
                placeholder="Find in page"
                spellcheck={false}
                onBlur={() => setFinding(false)}
                onKeyDown={(event) => {
                  if (event.key === "Enter") findInPage(event.currentTarget.value, event.shiftKey);
                  else if (event.key === "Escape") event.currentTarget.blur();
                }}
              />
              <span class="tag">find</span>
            </div>
          }
        >
          <div class="url" onClick={() => editing() || locate()}>
            <Show when={editing() || shownUrl() === undefined} fallback={<Address url={shownUrl()!} />}>
              <input
                ref={(input) => {
                  address = input;
                  queueMicrotask(() => editing() && (input.focus(), input.select()));
                }}
                value={untrack(shownUrl) ?? ""}
                placeholder="localhost:5173 or /path/to/page.html"
                spellcheck={false}
                onBlur={() => setEditing(false)}
                onKeyDown={(event) => {
                  if (event.key === "Enter") open(event.currentTarget.value);
                  else if (event.key !== "Escape") return;
                  event.currentTarget.blur();
                }}
              />
            </Show>
            <Show when={shown()?.tunnel}>
              {(port) => (
                <span class="tag live">
                  ⇄ {serverLabel(openView()!.at.server)}:{port()}
                </span>
              )}
            </Show>
          </div>
        </Show>
      </div>
      <div class="page" ref={surface}>
        <Show when={current() === undefined}>
          <div class="empty">
            Type a URL or a file's path to open a page, or have an agent run octopus browser open.
          </div>
        </Show>
        <Show when={shown()?.failed}>
          {(failed) => (
            <div class="empty failed">
              <div>
                <b>{nameOf(failed().url)}</b> could not load
              </div>
              <div class="why">{failed().error}</div>
              <code>{failed().url}</code>
              <button class="btn" onClick={() => navigate("reload")}>
                Try again
              </button>
            </div>
          )}
        </Show>
      </div>
    </div>
  );
}
