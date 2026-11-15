import { createEffect, createSignal, onCleanup, onMount, Show, untrack } from "solid-js";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { backend, on } from "@octopus/shared/backend.ts";
import { EVENTS, NEW_BROWSER_PANEL, type Downloaded } from "@octopus/shared/generated.ts";
import { Address } from "@octopus/shared/Address.tsx";
import {
  createPanelSelection,
  openPage,
  openTyped,
  pageName,
  panelName,
  type Target,
} from "@octopus/shared/browserPanels.ts";
import { browserPages, createPlacing, downloadNote, serially } from "@octopus/shared/browserPages.ts";
import { provideBrowser } from "@octopus/shared/commands.ts";
import { FolderTabs, type FolderTab } from "@octopus/shared/FolderTabs.tsx";
import type { BrowserId, BrowserPanel, Go, TaskView } from "@octopus/shared/generated.ts";
import { act, openView, report, serverLabel, showSurface } from "@octopus/shared/state.ts";
import { toast } from "@octopus/shared/Toast.tsx";
import type { OpenLink } from "@octopus/shared/Terminals.tsx";
import { desktop } from "./backend";
import { liftOnDrag } from "./lift";
import { cycled } from "./tabOrder";

const pages = browserPages({ load: desktop.browserLoad, forget: desktop.browserForget });
export const forgetBrowsersExcept = pages.forgetExcept;
act(
  on<Downloaded>(EVENTS.browserDownloaded)((downloaded) =>
    (downloaded.success ? toast : report)(downloadNote(downloaded)),
  ),
);
/** Per task key, the tabs this client closed, the last closed last, each with its page and its index. */
const closed = new Map<string, { url: string; at: number }[]>();
const [editing, setEditing] = createSignal(false);
/** While finding, the address field takes the text to find in the page, which it keeps for the next find. */
const [finding, setFinding] = createSignal(false);
const [sought, setSought] = createSignal("");
let address: HTMLInputElement | undefined;

/** Opens the tab this client closed last in the task again, at its index, and selects it. */
async function reopen(task: TaskView) {
  const last = closed.get(task.key)?.pop();
  if (last === undefined) return;
  const panel = await backend.browserOpen(task.at, NEW_BROWSER_PANEL, last.url);
  await backend.browserMove(task.at, panel, Math.min(last.at, task.task.browsers.length));
}

/** A terminal's link opens in the task's browser panel, the left side switching to Browser, or in the system's browser. */
export const openLink: OpenLink = (task, url, target) => {
  if (target === "system") act(desktop.openOutside(url));
  else {
    showSurface("browser");
    openPage(task, url);
  }
};

export function Browser(props: { visible: boolean }) {
  let surface!: HTMLDivElement;
  const panels = (): BrowserPanel[] => openView()?.task.browsers ?? [];
  const { target, panel: current, select } = createPanelSelection(openView);
  const shown = () => {
    const task = openView();
    const panel = current();
    return task === undefined || panel === undefined ? undefined : pages.page(task.key, panel.panel);
  };
  const shownUrl = () => shown()?.failed?.url || shown()?.url || current()?.url;

  createEffect(() => {
    const [task, chosen] = [openView(), current()];
    if (task !== undefined && chosen !== undefined && shown()?.load !== chosen.load)
      untrack(() => pages.load(task, chosen.panel, chosen.url, chosen.load));
  });

  const onScreen = () => {
    const task = openView();
    const panel = current();
    if (!props.visible || task === undefined || panel === undefined || shown() === undefined || shown()?.failed)
      return null;
    return { at: task.at, panel: panel.panel };
  };

  /**
   * A page arrives as this layer paints the frame it is laid out in, and leaves only once this layer has painted what
   * it uncovers: two processes paint the window, so a page may outstay its frame, but never leaves a hole.
   */
  const placing = createPlacing(() => {
    const { left, top, width, height } = surface.getBoundingClientRect();
    const shownNow = onScreen();
    return desktop.browserShowPage(shownNow && { ...shownNow, bounds: { x: left, y: top, width, height } });
  });
  createEffect(() => placing(onScreen() === null));
  onMount(() => {
    const observer = new ResizeObserver(() => placing(untrack(onScreen) === null));
    observer.observe(surface);
    onCleanup(() => observer.disconnect());
  });

  const open = (typed: string) => {
    const task = openView();
    if (task !== undefined) openTyped(task, typed);
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
      name: panelName(panel),
      where: undefined,
      title: panel.url,
      loading: pages.loading(openView()?.key ?? "", panel.panel),
      dirty: false,
    }));

  const close = (panel: BrowserId) => {
    const task = openView();
    if (task === undefined) return;
    const at = panels().findIndex((known) => known.panel === panel);
    const url = pages.page(task.key, panel)?.url || panels()[at]?.url;
    if (url !== undefined) closed.set(task.key, [...(closed.get(task.key) ?? []), { url, at }]);
    act(backend.browserClose(task.at, panel));
  };

  const move = async (panel: BrowserId, to: number) => {
    const task = openView();
    if (task !== undefined) await backend.browserMove(task.at, panel, to);
  };

  const choose = (panel: Target) => {
    select(panel);
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
                <b>{pageName(failed().url)}</b> could not load
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
