import { createResource, createSignal, onCleanup, Show } from "solid-js";
import { model, report } from "@octopus/shared/state.ts";
import { Icon } from "./Icon";
import { Circle, CrumbBar, Id, sectionColor } from "./parts";
import { phone } from "./phone";

/** The task's newest browser panel under its URL: the host in text, the path dim, "tunnel" while it reaches the server. */
export function Browser(props: { taskKey: string }) {
  const view = () => model.tasks[props.taskKey];
  const panel = () => view()?.task.browsers.at(-1);
  /** Each `load` of the panel, and each reload, loads its page again; another client's navigation leaves it. */
  const [loads, reload] = createSignal(0);
  const [page] = createResource(
    () => {
      const opened = panel();
      return opened === undefined ? false : `${opened.panel} ${opened.load} ${loads()}`;
    },
    () => {
      const shown = view()!;
      return phone.browse(shown.at.server, panel()!.url).catch((error) => report(error).then(() => undefined));
    },
  );
  onCleanup(() => phone.unbrowse().catch(report));
  const parts = () => {
    const url = new URL(panel()?.url ?? "about:blank");
    if (url.protocol === "file:") return { host: decodeURIComponent(url.pathname), path: "" };
    return { host: url.host, path: `${url.pathname}${url.search}`.replace(/^\/$/, "") };
  };
  return (
    <>
      <CrumbBar
        up="Task"
        color={sectionColor(view()?.section)}
        crumb={<span class="name">{view()?.task.title}</span>}
        title="Browser"
        end={<Circle icon="reload" label="Reload" tap={() => reload((count) => count + 1)} />}
      />
      <Show
        when={panel()}
        fallback={<div class="empty">no browser panel; an agent opens one with octopus browser open</div>}
      >
        <div class="urlrow">
          <div class="url">
            <Icon name="globe" />
            <span class="name">
              <b>{parts().host}</b>
              {parts().path}
            </span>
            <Show when={page()?.tunneled}>
              <Id class="live">tunnel</Id>
            </Show>
          </div>
        </div>
        <Show when={page()} keyed>
          {(shown) => <iframe class="web fill" src={shown.url} title="Browser" />}
        </Show>
      </Show>
    </>
  );
}
