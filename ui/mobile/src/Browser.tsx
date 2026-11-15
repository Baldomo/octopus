import { createEffect, createSignal, Match, onCleanup, onMount, Show, Switch, untrack } from "solid-js";
import { Address } from "@octopus/shared/Address.tsx";
import { backend } from "@octopus/shared/backend.ts";
import { browserOpened, browserPages, createPlacing, serially } from "@octopus/shared/browserPages.ts";
import { createPanelSelection, openTyped, pageName, panelName, type Target } from "@octopus/shared/browserPanels.ts";
import { NEW_BROWSER_PANEL, type BrowserId, type Go, type TaskRef } from "@octopus/shared/generated.ts";
import { act, model } from "@octopus/shared/state.ts";
import { feel } from "./feel";
import { Icon } from "./Icon";
import { claimBack, screen, sheet } from "./nav";
import { Circle, CrumbBar, FolderTabs, Gap, Id, Para, Secondary, sectionColor } from "./parts";
import { phone, phoneEvents } from "./phone";
import { sliding } from "./stack";
import { toasting } from "./Toast";

const pages = browserPages({ load: phone.browserLoad, forget: phone.browserForget });
export const forgetBrowsersExcept = pages.forgetExcept;
const [draws, setDraws] = createSignal<boolean>();

act(phone.browserEngine().then(setDraws));
act(
  browserOpened(({ task, url }) => {
    const view = model.tasks[task];
    if (view !== undefined) act(backend.browserOpen(view.at, NEW_BROWSER_PANEL, url));
  }),
);
act(phoneEvents.browserEnded(({ task, panel }) => pages.drop(task, panel)));

export function Browser(props: { taskKey: string }) {
  const view = () => model.tasks[props.taskKey];
  const { target, panel, select } = createPanelSelection(view);
  const page = () => {
    const chosen = panel();
    return chosen === undefined ? undefined : pages.page(props.taskKey, chosen.panel);
  };
  const address = () => page()?.failed?.url ?? (page()?.url || panel()?.url);
  const top = () => {
    const shown = screen();
    return shown.at === "browser" && shown.key === props.taskKey;
  };

  createEffect(() => {
    const [task, chosen] = [view(), panel()];
    if (draws() && task !== undefined && chosen !== undefined && page()?.load !== chosen.load)
      untrack(() => pages.load(task, chosen.panel, chosen.url, chosen.load));
  });

  const onPage = (call: (at: TaskRef, panel: BrowserId) => Promise<void>) => {
    const [task, chosen] = [view(), panel()];
    if (task !== undefined && chosen !== undefined && page() !== undefined)
      act(serially(() => call(task.at, chosen.panel)));
  };
  const go = (step: Go) => onPage((at, chosen) => phone.browserGo(at, chosen, step));
  const seek = (text: string, backwards: boolean) =>
    onPage((at, chosen) => phone.browserFind(at, chosen, text, backwards));
  const reload = () => {
    const [task, chosen, failed] = [view(), panel(), page()?.failed];
    if (task !== undefined && chosen !== undefined && failed) pages.load(task, chosen.panel, failed.url, chosen.load);
    else go("reload");
  };

  createEffect(() => {
    if (top() && page()?.back) onCleanup(claimBack(() => go("back")));
  });

  let area!: HTMLDivElement;
  const onScreen = () => {
    const [task, chosen] = [view(), panel()];
    if (!top() || task === undefined || chosen === undefined || page() === undefined || page()?.failed) return null;
    return { at: task.at, panel: chosen.panel, covered: sliding() || sheet() !== null };
  };
  const [clear, setClear] = createSignal(false);
  const still = () => onScreen() !== null && !sliding();
  const placing = createPlacing(async (latest) => {
    const shown = onScreen();
    const box = area.getBoundingClientRect();
    const app = area.closest<HTMLElement>(".app")!;
    const around = app.getBoundingClientRect();
    const edges = {
      top: (box.top - around.top) / around.height,
      bottom: (around.bottom - box.bottom) / around.height,
      left: (box.left - around.left) / around.width,
      right: (around.right - box.right) / around.width,
    };
    for (const [edge, share] of Object.entries(edges)) app.style.setProperty(`--hole-${edge}`, `${share * 100}%`);
    const scale = innerWidth / around.width;
    const bounds = { x: box.left * scale, y: box.top * scale, width: box.width * scale, height: box.height * scale };
    await phone.browserPlace(shown && { at: shown.at, panel: shown.panel, bounds }, shown?.covered ?? false);
    if (latest() && still()) setClear(true);
  });
  const place = () => {
    if (!still()) setClear(false);
    placing(false);
  };
  createEffect(place);
  onMount(() => {
    const observer = new ResizeObserver(place);
    observer.observe(area);
    onCleanup(() => observer.disconnect());
  });
  onCleanup(() => act(serially(() => phone.browserPlace(null, false))));

  /** The field stays in the page, drawn behind the address until it has the focus, so the tap that brings it up raises the keyboard. */
  let field!: HTMLInputElement;
  const [editing, setEditing] = createSignal(false);
  const [finding, setFinding] = createSignal(false);
  const [sought, setSought] = createSignal("");
  const locate = (typed: string, find: boolean) => {
    setFinding(find);
    field.value = typed;
    field.focus();
    field.select();
  };
  /** A field the app loses with its webview's focus stays the active element, and would take the focus back with it. */
  const leave = () => document.activeElement === field && field.blur();
  const step = (backwards: boolean) => {
    setSought(field.value);
    if (field.value !== "") seek(field.value, backwards);
  };
  const open = (typed: string) => {
    const task = view();
    if (task !== undefined) openTyped(task, typed);
  };
  const choose = (next: Target) => {
    if (next !== target()) feel("select");
    select(next);
    if (next === NEW_BROWSER_PANEL) locate("", false);
  };
  const close = (closing: string) => {
    const task = view();
    if (task !== undefined) act(backend.browserClose(task.at, closing));
  };

  return (
    <>
      <CrumbBar
        up="Task"
        color={sectionColor(view()?.section)}
        crumb={<span class="name">{view()?.task.title}</span>}
        title="Browser"
        end={
          <div class="right">
            <Circle icon="search" label="Find in page" tap={() => locate(sought(), true)} />
            <Circle icon="reload" label="Reload" tap={reload} />
          </div>
        }
      />
      <Show when={(view()?.task.browsers.length ?? 0) > 0}>
        <FolderTabs
          tabs={(view()?.task.browsers ?? []).map((each) => ({ key: each.panel, name: panelName(each) }))}
          selected={target()}
          choose={choose}
          close={close}
          add={{
            label: "New browser panel",
            selected: target() === NEW_BROWSER_PANEL,
            run: () => choose(NEW_BROWSER_PANEL),
          }}
        />
      </Show>
      <div class="urlrow">
        <div class="capsule">
          <button class="circle" aria-label="Back" disabled={!page()?.back} onClick={() => go("back")}>
            <Icon name="back" big />
          </button>
          <button class="circle" aria-label="Forward" disabled={!page()?.forward} onClick={() => go("forward")}>
            <Icon name="fwd" big />
          </button>
        </div>
        <div class="url" onClick={() => editing() || locate(address() ?? "", false)}>
          <span class="slot">
            <Icon name={finding() ? "search" : "globe"} />
          </span>
          <input
            ref={field}
            classList={{ behind: !editing() && panel() !== undefined }}
            placeholder={finding() ? "Find in page" : "localhost:5173 or /path/to/page.html"}
            spellcheck={false}
            autocapitalize="off"
            autocorrect="off"
            autocomplete="off"
            inputmode={finding() ? "text" : "url"}
            enterkeyhint={finding() ? "search" : "go"}
            onFocus={() => setEditing(true)}
            onBlur={() => {
              setEditing(false);
              leave();
              if (finding()) seek("", false);
              setFinding(false);
            }}
            onKeyDown={(event) => {
              if (event.key !== "Enter") return;
              event.preventDefault();
              if (finding()) return step(false);
              open(event.currentTarget.value);
              event.currentTarget.blur();
            }}
          />
          <Show when={!editing() && address()}>{(url) => <Address url={url()} />}</Show>
          <Show when={!editing() && page()?.tunnel}>
            <Id class="live">tunnel</Id>
          </Show>
          <Show when={finding()}>
            <div class="stepper">
              <span role="button" aria-label="Previous match" onClick={() => step(true)}>
                <Icon name="up" />
              </span>
              <span role="button" aria-label="Next match" onClick={() => step(false)}>
                <Icon name="car" />
              </span>
            </div>
          </Show>
        </div>
      </div>
      <div class="page fill">
        <Switch>
          <Match when={panel() === undefined}>
            <div class="empty">no page open; type one or run octopus browser open</div>
          </Match>
          <Match when={draws() === false}>
            <div class="empty">Android System WebView needs an update</div>
          </Match>
          <Match when={page()?.failed}>
            {(failed) => (
              <>
                <Gap n={2} />
                <Para>
                  <b>{pageName(failed().url)}</b> could not load: {failed().error}
                </Para>
                <Para>
                  <span class="failed-url">{failed().url}</span>
                </Para>
                <Gap n={2} />
                <Secondary tap={reload}>Try again</Secondary>
              </>
            )}
          </Match>
        </Switch>
        <div class="hole" classList={{ clear: clear() }} ref={area} />
        <Show when={toasting()}>
          <div class="toastroom" />
        </Show>
      </div>
    </>
  );
}
