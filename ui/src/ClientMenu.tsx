import { createSignal, For, onMount, Show } from "solid-js";
import type { Appearance, ServerView } from "./generated";
import { commands, hint } from "./keys";
import { closePopover, dismissible, Head, Item, Sep } from "./Menu";
import { ServerMenu } from "./SidebarControls";
import { VERSION } from "./generated";
import { me, report, servers, setAppearance, showClientMenu, ui, zoom } from "./state";

const APPEARANCES: [Appearance, string][] = [
  ["system", "System"],
  ["light", "Light"],
  ["dark", "Dark"],
];
const COPIED_MS = 1800;

function ServerRow(props: { server: ServerView; open: () => void }) {
  return (
    <button class="mi" role="menuitem" onClick={props.open}>
      <span class="dot" classList={{ off: !props.server.reachable }} />
      {props.server.label}
      <span class="rtt">{props.server.reachable ? `${props.server.rtt_ms ?? "–"} ms` : "unreachable"}</span>
    </button>
  );
}

/** Hangs from the title bar's gear, and shows a server's menu in its place once that server is chosen. */
export function ClientMenu(props: { floats: (element: HTMLElement) => void; shown: boolean }) {
  let element!: HTMLDivElement;
  const [server, setServer] = createSignal<string | null>(null);
  const [copied, setCopied] = createSignal(false);
  const about = () =>
    navigator.clipboard.writeText(`octopus ${VERSION}`).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), COPIED_MS);
    }, report);
  onMount(() => {
    closePopover();
    dismissible(element, () => showClientMenu(false));
  });
  return (
    <div
      ref={(menu) => {
        element = menu;
        props.floats(menu);
      }}
      class="menu wide client-menu"
      classList={{ hidden: !props.shown }}
      role="menu"
    >
      <Show
        when={server()}
        keyed
        fallback={
          <>
            <Head title={me()} lines={[]} />
            <div class="mh label">
              <span>Servers</span>
            </div>
            <For each={servers()}>{(view) => <ServerRow server={view} open={() => setServer(view.name)} />}</For>
            <Item run={() => commands.pairServer.run()}>
              <span class="glyph">+</span>Pair a server…
            </Item>
            <Sep />
            <div class="mrow">
              Appearance
              <div class="seg">
                <For each={APPEARANCES}>
                  {([appearance, label]) => (
                    <button
                      classList={{ on: ui.prefs.appearance === appearance }}
                      onClick={() => setAppearance(appearance)}
                    >
                      {label}
                    </button>
                  )}
                </For>
              </div>
            </div>
            <div class="mrow">
              Zoom
              <span class="stepper">
                <button class="mini" title={hint(commands.zoomOut)} onClick={() => zoom("out")}>
                  −
                </button>
                <output>{ui.prefs.zoom}%</output>
                <button class="mini" title={hint(commands.zoomIn)} onClick={() => zoom("in")}>
                  +
                </button>
              </span>
            </div>
            <Sep />
            <Item kbd={hint(commands.shortcuts)} run={commands.shortcuts.run}>
              Keyboard shortcuts
            </Item>
            <button class="mi" role="menuitem" onClick={about}>
              About octopus<span class="why">{copied() ? "copied" : VERSION}</span>
            </button>
            <Item kbd={hint(commands.quit)} run={commands.quit.run}>
              Quit octopus
            </Item>
          </>
        }
      >
        {(name) => <ServerMenu name={name} />}
      </Show>
    </div>
  );
}
