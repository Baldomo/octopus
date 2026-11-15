import { onCleanup } from "solid-js";
import { createStore, produce } from "solid-js/store";
import { backend, on } from "./backend.ts";
import { openedAt } from "./browserPanels.ts";
import { said } from "./errors.ts";
import {
  BACKGROUND_BROWSER_PANEL,
  EVENTS,
  NEW_BROWSER_PANEL,
  type BrowserId,
  type Downloaded,
  type Failed,
  type Opened,
  type Paged,
  type TaskRef,
  type TaskView,
} from "./generated.ts";
import { act, model } from "./state.ts";

/** What draws a browser panel's page on this client, in a native view of its own. */
export type BrowserEngine = {
  load: (at: TaskRef, panel: BrowserId, url: string) => Promise<void>;
  forget: (at: TaskRef, panel: BrowserId) => Promise<void>;
};

/** A kept panel's page on this client, which moves off its server `url` as the user navigates. */
export type Page = Omit<Paged, "task" | "panel"> & {
  failed: { url: string; error: string } | null;
  /** The panel's `load` this client last loaded. */
  load: number;
};

/** Native calls run in the order the UI makes them, so a view exists before it is placed or let go. */
let queue: Promise<unknown> = Promise.resolve();
export const serially = <T>(call: () => Promise<T>): Promise<T> => {
  const next = queue.then(call);
  queue = next.catch(() => undefined);
  return next;
};

/**
 * Places the page once the frame is laid out and every native call before has settled, unless a later placing
 * superseded it; `place` hears whether it still is the latest. A page leaving waits for the frame it uncovers to paint.
 */
export function createPlacing(place: (latest: () => boolean) => Promise<unknown>) {
  let latest = 0;
  onCleanup(() => latest++);
  return (leaving: boolean) => {
    const placing = ++latest;
    const current = () => placing === latest;
    const run = () => act(serially(async () => current() && (await place(current))));
    requestAnimationFrame(leaving ? () => setTimeout(run) : run);
  };
}

/** The pages this client keeps, per task key, which `engine` draws, and the tabs their pages open. */
export function browserPages(engine: BrowserEngine) {
  const [kept, setKept] = createStore<Record<string, { at: TaskRef; pages: Record<BrowserId, Page> }>>({});
  const page = (task: string, panel: BrowserId): Page | undefined => kept[task]?.pages[panel];
  const loading = (task: string, panel: BrowserId) => page(task, panel)?.loading === true;
  const setPage = (task: string, panel: BrowserId, change: Partial<Page>) =>
    setKept(
      produce((all) => {
        const known = all[task]?.pages[panel];
        if (known !== undefined) Object.assign(known, change);
      }),
    );
  const drop = (task: string, panel: BrowserId) => setKept(produce((all) => delete all[task]?.pages[panel]));
  /** A failed page shows why in place of the page, which then reaches no tunnel. */
  const fail = (task: string, panel: BrowserId, failed: { url: string; error: string }) =>
    setPage(task, panel, { failed, loading: false, tunnel: null });

  act(
    on<Paged>(EVENTS.browserPaged)(({ task, panel, ...paged }) =>
      setPage(task, panel, { ...paged, ...(paged.loading ? { failed: null } : {}) }),
    ),
  );
  /** A failure stands until its page loads again, so a load that fails late leaves the newer failure it trails. */
  act(
    on<Failed>(EVENTS.browserFailed)(({ task, panel, url, error }) => {
      if (!page(task, panel)?.failed) fail(task, panel, { url, error });
    }),
  );

  /** Loads `url` in the panel's view on this client alone, as its `load`. */
  const load = (task: TaskView, panel: BrowserId, url: string, nth: number) => {
    setKept(
      produce((all) => {
        const pages = (all[task.key] ??= { at: task.at, pages: {} }).pages;
        pages[panel] = {
          back: false,
          forward: false,
          tunnel: null,
          ...pages[panel],
          url,
          loading: true,
          failed: null,
          load: nth,
        };
      }),
    );
    act(
      serially(() => engine.load(task.at, panel, url)).catch((error: unknown) =>
        fail(task.key, panel, { url, error: said(error) }),
      ),
    );
  };

  /** Per task key, the panel whose page opened each panel a page opened on this client. */
  const openers = new Map<string, Map<BrowserId, BrowserId>>();

  /** Lets go of the views and openers of the tasks not among `keep`, and of every panel its server closed. */
  const forgetExcept = (keep: string[]) => {
    for (const [task, { at, pages }] of Object.entries(kept)) {
      const open = keep.includes(task) ? (model.tasks[task]?.task.browsers ?? []) : [];
      for (const panel of Object.keys(pages).filter((panel) => !open.some((known) => known.panel === panel))) {
        drop(task, panel);
        act(serially(() => engine.forget(at, panel)));
      }
    }
    for (const task of openers.keys()) if (!keep.includes(task)) openers.delete(task);
  };

  /**
   * Opens `url` in a new tab after the `opener`'s tab and the tabs it opened before. A tab opened behind loads on this
   * client at once, and no client selects it.
   */
  const openFrom = async (task: TaskView, opener: BrowserId, url: string, select: boolean) => {
    const panel = await backend.browserOpen(task.at, select ? NEW_BROWSER_PANEL : BACKGROUND_BROWSER_PANEL, url);
    const opened = openers.get(task.key) ?? new Map<BrowserId, BrowserId>();
    openers.set(task.key, opened.set(panel, opener));
    if (!select) load(task, panel, url, 0);
    const others = (model.tasks[task.key]?.task.browsers ?? [])
      .map((known) => known.panel)
      .filter((id) => id !== panel);
    await backend.browserMove(
      task.at,
      panel,
      openedAt(others, opener, (tab) => opened.get(tab)),
    );
  };
  act(
    on<Opened>(EVENTS.browserOpened)(({ task, panel, url, select }) => {
      const view = model.tasks[task];
      if (view !== undefined) act(openFrom(view, panel, url, select));
    }),
  );

  return { page, loading, load, drop, forgetExcept, openFrom };
}

/** What a toast says of a download that ended. */
export const downloadNote = ({ file, folder, success }: Downloaded) =>
  success ? `Saved ${file} to ${folder}` : `The download of ${file} failed`;
