import { createResource, createSignal, onCleanup, Show } from "solid-js";
import { model, report } from "@octopus/shared/state.ts";
import { Icon } from "./Icon";
import { back } from "./nav";
import { Circle, Id, sectionColor } from "./parts";
import { phone } from "./phone";

/** The task's browser panel under its URL: the host in text, the path dim, "tunnel" while it reaches the server. */
export function Browser(props: { taskKey: string }) {
  const view = () => model.tasks[props.taskKey];
  const panel = () => view()?.task.browsers[0];
  const [page] = createResource(
    () => {
      const shown = view();
      const url = panel()?.url;
      return shown && url ? { server: shown.at.server, url } : false;
    },
    ({ server, url }) => phone.browse(server, url).catch((error) => report(error).then(() => undefined)),
  );
  onCleanup(() => phone.unbrowse().catch(report));
  const [loads, reload] = createSignal(0);
  const parts = () => {
    const url = new URL(page()?.url ?? panel()?.url ?? "about:blank");
    return { host: url.host, path: `${url.pathname}${url.search}`.replace(/^\/$/, "") };
  };
  return (
    <>
      <div class="bar">
        <Circle icon="back" label="Task" tap={back} />
        <div class="ttl" style={{ "--ws": sectionColor(view()?.section) }}>
          <span class="crumb">
            <i />
            <span class="name">{view()?.task.title}</span>
          </span>
          <b class="name">Browser</b>
        </div>
        <Circle icon="reload" label="Reload" tap={() => reload((count) => count + 1)} />
      </div>
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
          {(shown) => (
            <Show when={loads() + 1} keyed>
              <iframe class="web fill" src={shown.url} title="Browser" />
            </Show>
          )}
        </Show>
      </Show>
    </>
  );
}
