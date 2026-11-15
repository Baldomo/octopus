import inter from "@fontsource-variable/inter/files/inter-latin-opsz-normal.woff2?url";
import { createEffect, createSignal, For, Match, on, Show, Switch } from "solid-js";
import "virtual:design.css";
import "./app.css";
import { backend, events } from "@octopus/shared/backend.ts";
import { loadFonts } from "@octopus/shared/fonts.ts";
import { mount } from "@octopus/shared/mount.ts";
import type { Appearance } from "@octopus/shared/generated.ts";
import {
  act,
  applyBoot,
  applyModel,
  applyUi,
  report,
  servers,
  setAccount,
  setServers,
  ui,
} from "@octopus/shared/state.ts";
import { applyFilesChanged, forgetFilesExcept } from "@octopus/shared/taskFiles.ts";
import { Board } from "./Board";
import { followFront } from "./front";
import { steadyCaret } from "./caret";
import { Browser, forgetBrowsersExcept, LinkMenu } from "./Browser";
import { ClientMenu, ServerMenu } from "./ClientMenu";
import { EditorScreen, Files } from "./Files";
import { Sprite } from "./Icon";
import { loadSymbols } from "./symbols";
import { FileMenu } from "./FileMenu";
import { FolderSheet } from "./FolderSheet";
import type { Toward } from "@octopus/shared/motion.ts";
import { setMicDenied, sheetEntries, type Screen, type Sheet, type SheetEntry } from "./nav";
import { NewTask } from "./NewTask";
import { PairSheet, Scan, scanning, Welcome } from "./Pairing";
import { phone } from "./phone";
import { Search } from "./Search";
import { SheetPresence, type Drawn } from "./Sheet";
import { Stack, sliding } from "./stack";
import { Task } from "./Task";
import { Reopen, TaskMenu } from "./TaskMenu";
import { Tasks } from "./Tasks";
import { Toast } from "./Toast";
import { followViewport } from "./viewport";
import { followTouches } from "./touch";
import { followMotion } from "./motion";
import { AddWorkspace, SectionMenu } from "./Workspaces";

const KEPT_BROWSER_TASKS = 4;

const SCHEMES: Record<Appearance, string> = { system: "light dark", light: "light", dark: "dark" };

function Shown(props: { screen: Screen }) {
  return (
    <Switch>
      <Match when={props.screen.at === "tasks"}>
        <Show when={servers().length > 0} fallback={<Welcome />}>
          <Tasks />
        </Show>
      </Match>
      <Match when={props.screen.at === "scan" && props.screen}>{(scan) => <Scan reading={scan().reading} />}</Match>
      <Match when={props.screen.at === "task" && props.screen.key} keyed>
        {(key) => <Task taskKey={key} />}
      </Match>
      <Match when={props.screen.at === "board" && props.screen.section} keyed>
        {(section) => <Board section={section} />}
      </Match>
      <Match when={props.screen.at === "files" && props.screen.key} keyed>
        {(key) => <Files taskKey={key} />}
      </Match>
      <Match when={props.screen.at === "editor" && props.screen.key} keyed>
        {(key) => <EditorScreen taskKey={key} />}
      </Match>
      <Match when={props.screen.at === "browser" && props.screen.key} keyed>
        {(key) => <Browser taskKey={key} />}
      </Match>
      <Match when={props.screen.at === "search"}>
        <Search />
      </Match>
    </Switch>
  );
}

function SheetPage(props: { sheet: Sheet }) {
  const shown = props.sheet;
  return (
    <Switch>
      <Match when={shown.is === "client"}>
        <ClientMenu />
      </Match>
      <Match when={shown.is === "server" && shown.name}>{(name) => <ServerMenu name={name()} />}</Match>
      <Match when={shown.is === "pair" && shown}>
        {(read) => <PairSheet invite={read().invite} reading={read().reading} />}
      </Match>
      <Match when={shown.is === "task" && shown}>
        {(menu) => <TaskMenu taskKey={menu().key} open={menu().open} />}
      </Match>
      <Match when={shown.is === "reopen" && shown.key}>{(key) => <Reopen taskKey={key()} />}</Match>
      <Match when={shown.is === "entry" && shown}>
        {(entry) => <FileMenu taskKey={entry().key} path={entry().path} kind={entry().kind} />}
      </Match>
      <Match when={shown.is === "link" && shown}>
        {(held) => <LinkMenu taskKey={held().key} panel={held().panel} link={held().link} />}
      </Match>
      <Match when={shown.is === "section" && shown.name}>{(name) => <SectionMenu name={name()} />}</Match>
      <Match when={shown.is === "addws"}>
        <AddWorkspace />
      </Match>
      <Match when={shown.is === "newtask" && shown}>{(form) => <NewTask section={form().section} />}</Match>
      <Match when={shown.is === "folder" && shown}>{(folder) => <FolderSheet picker={folder().picker} />}</Match>
    </Switch>
  );
}

type ShownSheet = {
  entry: SheetEntry;
  replacing: { toward: Toward; drawn: Drawn } | null;
  over: Drawn | null;
  drawn: Drawn;
  leaving: () => boolean;
  leave: () => void;
};

/** The sheets up, the lowest first, and each one still on its way down. */
const [shown, setShown] = createSignal<ShownSheet[]>([]);

/** What carries the top sheet back, while one is up and staying. */
const sheetBack = () => shown().findLast((each) => !each.leaving())?.drawn.back ?? null;

/**
 * Draws the sheets up. A sheet at a depth no sheet held rises, over the one below it; one shown in place of another
 * replaces it at once; one no longer up lowers.
 */
function Sheets() {
  const make = (entry: SheetEntry, replaced: ShownSheet | undefined, under: ShownSheet | undefined): ShownSheet => {
    const [leaving, setLeaving] = createSignal(false);
    const replacing = replaced && entry.turned ? { toward: entry.turned, drawn: replaced.drawn } : null;
    return {
      entry,
      replacing,
      over: replacing ? null : (under?.drawn ?? null),
      drawn: { panel: null, page: null, height: 0, scrolled: 0, back: null },
      leaving,
      leave: () => setLeaving(true),
    };
  };
  createEffect(
    on(sheetEntries, (now, before = []) => {
      let next = shown();
      const upAt = (entry: SheetEntry | undefined) => next.find((each) => each.entry === entry && !each.leaving());
      for (let depth = 0; depth < Math.max(now.length, before.length); depth++) {
        const [was, is] = [before[depth], now[depth]];
        if (was === is) continue;
        const up = upAt(was);
        if (is === undefined) up?.leave();
        else next = [...next.filter((each) => each !== up), make(is, up, upAt(now[depth - 1]))];
      }
      setShown(next);
    }),
  );
  const gone = (left: ShownSheet) => setShown((all) => all.filter((each) => each !== left));
  return (
    <For each={shown()}>
      {(each) => (
        <SheetPresence.Provider
          value={{
            replacing: each.replacing,
            over: each.over,
            drawn: each.drawn,
            leaving: each.leaving,
            gone: () => gone(each),
          }}
        >
          <SheetPage sheet={each.entry.sheet} />
        </SheetPresence.Provider>
      )}
    </For>
  );
}

const redial = () => act(phone.redial());

/** One network change fires several events, and each redial ends the dials under way. */
const REDIAL_SETTLE_MS = 1000;
let settling: ReturnType<typeof setTimeout> | undefined;
const redialOnceSettled = () => {
  clearTimeout(settling);
  settling = setTimeout(redial, REDIAL_SETTLE_MS);
};

/**
 * The network can change with the app in front too. Android's webview reports the connection's type, whose `change`
 * also fires as its speed estimates move; `online` is all iOS's reports.
 */
const followNetwork = () => {
  window.addEventListener("online", redialOnceSettled);
  const connection = (navigator as Navigator & { connection?: EventTarget & { type: string } }).connection;
  if (connection === undefined) return;
  let type = connection.type;
  connection.addEventListener("change", () => {
    if (connection.type !== type) redialOnceSettled();
    type = connection.type;
  });
};

/** Every pushed screen stays mounted under the one on top, so going back finds it as it was left. */
function App() {
  createEffect(() => document.documentElement.style.setProperty("color-scheme", SCHEMES[ui.prefs.appearance]));
  createEffect(() => document.documentElement.toggleAttribute("data-scanning", scanning() && !sliding()));
  createEffect(on(() => [...ui.recent], forgetFilesExcept));
  createEffect(() => forgetBrowsersExcept(ui.recent.slice(0, KEPT_BROWSER_TASKS)));
  return (
    <>
      <Sprite />
      <Stack sheetBack={sheetBack}>{(screen) => <Shown screen={screen} />}</Stack>
      <Sheets />
      <Toast />
    </>
  );
}

async function start() {
  steadyCaret();
  followViewport();
  followFront();
  followNetwork();
  followTouches();
  followMotion();
  await Promise.all([loadFonts(inter), loadSymbols()]);
  await Promise.all([
    events.model(applyModel),
    events.servers(setServers),
    events.account(setAccount),
    events.ui(applyUi),
    events.microphoneDenied(setMicDenied),
    events.filesChanged(applyFilesChanged),
  ]);
  applyBoot(await backend.boot());
  mount(() => <App />, document.body);
}

start()
  .catch((error) => {
    const problem = `octopus could not start: ${error}`;
    document.body.textContent = problem;
    report(problem);
  })
  .finally(() => requestAnimationFrame(() => backend.ready().catch(console.error)));
