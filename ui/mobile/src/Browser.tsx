import { createResource, createSignal, onCleanup, Show } from "solid-js";
import { Address } from "@octopus/shared/Address.tsx";
import { backend } from "@octopus/shared/backend.ts";
import { createPanelSelection, openTyped, pageName, panelName, type Target } from "@octopus/shared/browserPanels.ts";
import { said } from "@octopus/shared/errors.ts";
import { NEW_BROWSER_PANEL } from "@octopus/shared/generated.ts";
import { act, model } from "@octopus/shared/state.ts";
import { feel } from "./feel";
import { Icon } from "./Icon";
import { Circle, CrumbBar, FolderTabs, Gap, Id, Para, Secondary, sectionColor } from "./parts";
import { phone } from "./phone";

/** The task's browser panels as tabs over the selected one's address and its page, "tunnel" while it reaches the server. */
export function Browser(props: { taskKey: string }) {
  const view = () => model.tasks[props.taskKey];
  const { target, panel, select } = createPanelSelection(view);
  const address = () => panel()?.url;

  /** Each `load` of the panel, each panel chosen and each reload loads its page again; another client's navigation leaves it. */
  const [loads, reload] = createSignal(0);
  const [page] = createResource(
    () => {
      const opened = panel();
      return opened === undefined ? false : `${opened.panel} ${opened.load} ${loads()}`;
    },
    () => phone.browse(view()!.at.server, address()!).catch((error: unknown) => ({ error: said(error) })),
  );
  const settled = () => (page.loading ? undefined : page());
  const shown = () => {
    const now = settled();
    return now !== undefined && "url" in now ? now : undefined;
  };
  const failed = () => {
    const now = settled();
    return now !== undefined && "error" in now ? now.error : undefined;
  };
  const again = () => reload((count) => count + 1);
  onCleanup(() => act(phone.unbrowse()));

  /** The address is a field always, drawn as the page's address until it has the focus, so a tap raises the keyboard. */
  let field!: HTMLInputElement;
  const [editing, setEditing] = createSignal(false);
  const locate = (typed: string) => {
    field.value = typed;
    field.focus();
    field.select();
  };
  const open = (typed: string) => {
    const task = view();
    if (task !== undefined) openTyped(task, typed);
  };
  const choose = (next: Target) => {
    if (next !== target()) feel("select");
    select(next);
    if (next === NEW_BROWSER_PANEL) locate("");
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
        end={<Circle icon="reload" label="Reload" tap={again} />}
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
        <div class="url" onClick={() => editing() || locate(address() ?? "")}>
          <span class="slot">
            <Icon name="globe" />
          </span>
          <input
            ref={field}
            classList={{ behind: !editing() && address() !== undefined }}
            placeholder="localhost:5173 or /path/to/page.html"
            spellcheck={false}
            autocapitalize="off"
            autocorrect="off"
            autocomplete="off"
            inputmode="url"
            enterkeyhint="go"
            onFocus={() => setEditing(true)}
            onBlur={() => setEditing(false)}
            onKeyDown={(event) => {
              if (event.key !== "Enter") return;
              event.preventDefault();
              open(event.currentTarget.value);
              event.currentTarget.blur();
            }}
          />
          <Show when={!editing() && address()}>{(url) => <Address url={url()} />}</Show>
          <Show when={!editing() && shown()?.tunneled}>
            <Id class="live">tunnel</Id>
          </Show>
        </div>
      </div>
      <Show when={panel()} fallback={<div class="empty">no page open; type one or run octopus browser open</div>}>
        <Show when={failed()}>
          {(why) => (
            <>
              <Gap n={2} />
              <Para>
                <b>{pageName(address()!)}</b> could not load: {why()}
              </Para>
              <Gap n={2} />
              <Secondary tap={again}>Try again</Secondary>
            </>
          )}
        </Show>
        <Show when={shown()} keyed>
          {(loaded) => <iframe class="web fill" src={loaded.url} title="Browser" />}
        </Show>
      </Show>
    </>
  );
}
