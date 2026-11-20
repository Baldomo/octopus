import { createMemo, createResource, createSignal, For, onMount, Show, type JSX } from "solid-js";
import { backend } from "./backend";
import { commands } from "./commands";
import { baseName, folderPicker } from "./folderPath";
import type { Swatch } from "./generated";
import { anyMenuOpen, dismissible } from "./Menu";
import { floatPanel } from "./motion";
import { act, model, serverLabel, servers } from "./state";
import { FolderPick } from "./NewTask";
import { pickSwatch, sameSwatch, SWATCHES } from "./swatches";
import { swatchColor } from "./tokens";
import { toast } from "./Toast";

type Renamed = { kind: "server" | "section" | "task"; key: string };
const [renaming, setRenaming] = createSignal<Renamed | null>(null);
export const startRename = (kind: Renamed["kind"], key: string) => setRenaming({ kind, key });
export const isRenaming = (kind: Renamed["kind"], key: string) => renaming()?.kind === kind && renaming()?.key === key;

export function RenameInput(props: { value: string; apply: (name: string) => Promise<unknown> }) {
  const stop = (event: Event) => event.stopPropagation();
  return (
    <input
      class="rename"
      spellcheck={false}
      value={props.value}
      ref={(input) =>
        requestAnimationFrame(() => {
          input.focus();
          input.select();
        })
      }
      onPointerDown={stop}
      onClick={stop}
      onDblClick={stop}
      onKeyDown={(event) => {
        stop(event);
        if (event.key === "Escape") setRenaming(null);
        if (event.key !== "Enter") return;
        const name = event.currentTarget.value.trim();
        setRenaming(null);
        if (name !== "" && name !== props.value) act(props.apply(name));
      }}
      onBlur={() => setRenaming(null)}
    />
  );
}

export function Matrix(props: { current: Swatch; auto: Swatch; pick: (swatch: Swatch | null) => void }) {
  return (
    <div class="matrix">
      <For each={SWATCHES}>
        {(swatch) => (
          <button
            type="button"
            class="swatch"
            classList={{ on: sameSwatch(swatch, props.current), auto: sameSwatch(swatch, props.auto) }}
            style={{ "--c": swatchColor(swatch) }}
            title={`${swatch.hue} ${swatch.tone}${sameSwatch(swatch, props.auto) ? " · auto" : ""}`}
            onClick={() => props.pick(pickSwatch(swatch, props.auto))}
          />
        )}
      </For>
    </div>
  );
}

const [adding, setAdding] = createSignal(false);

export const holdsKeyboard = () => anyMenuOpen() || renaming() !== null || adding();

/** "+ workspace", which raises the folder picker in its place. */
export function AddWorkspace() {
  return (
    <div class="anchor">
      <button class="addws" onClick={() => setAdding(true)}>
        <span>+</span>workspace
      </button>
      <Show when={adding()}>
        <AddPick />
      </Show>
    </div>
  );
}

/** This machine's server while it is reachable, else the first reachable server. */
const startingServer = () =>
  (
    servers().find((server) => server.reachable && server.relay === null) ??
    servers().find((server) => server.reachable)
  )?.name;

/**
 * The folder picker on a reachable server's home folder, the reachable servers to switch between above its path.
 * Choosing a folder adds a workspace named after it, rooted in it, colored auto; a name the server holds is refused
 * in the picker, and the server's refusal shows as a toast. While no server answers it offers pairing one, and it
 * opens on the first server that answers.
 */
function AddPick() {
  const [picked, setServer] = createSignal<string>();
  const server = createMemo<string | undefined>((kept) => picked() ?? kept ?? startingServer());
  const [refusal, setRefusal] = createSignal<string | null>(null);
  const [home] = createResource(server, (on) => backend.folderGet(on, "~"));
  const add = (on: string, folder: string) => {
    const name = baseName(folder);
    if (model.lists.workspaces.some((workspace) => workspace.server === on && workspace.name === name)) {
      setRefusal(`${serverLabel(on)} already has ${name}.`);
      return false;
    }
    backend.workspaceAdd(on, name, folder, null).then(
      () =>
        toast(
          <>
            Added <b>{name}</b> on {serverLabel(on)}
          </>,
        ),
      (refused) => toast(String(refused)),
    );
  };
  const head = (
    <div class="pick servers-pick">
      <For each={servers()}>
        {(option) => (
          <button
            type="button"
            class="host"
            classList={{ on: server() === option.name }}
            disabled={!option.reachable}
            title={option.reachable ? undefined : "unreachable"}
            onClick={() => {
              setRefusal(null);
              setServer(option.name);
            }}
          >
            <span class="dot" classList={{ off: !option.reachable }} />
            <span class="name">{option.label}</span>
          </button>
        )}
      </For>
    </div>
  );
  return (
    <Show when={server() !== undefined} fallback={<NoServer head={head} />}>
      <Show when={home.state === "ready" && { on: server()!, root: home().folder }} keyed>
        {({ on, root }) => (
          <FolderPick
            picker={folderPicker(on, root, "", (folder) => add(on, folder))}
            gone={() => setAdding(false)}
            head={head}
            refusal={refusal()}
          />
        )}
      </Show>
    </Show>
  );
}

/** The picker's place while no server answers: what the user can do, and the button that does it. */
function NoServer(props: { head: JSX.Element }) {
  let panel!: HTMLDivElement;
  let leaving = false;
  const leave = (then?: () => void) => {
    if (leaving) return;
    leaving = true;
    const gone = () => {
      setAdding(false);
      then?.();
    };
    void floatPanel(panel, true).then(gone, gone);
  };
  onMount(() => {
    void floatPanel(panel);
    dismissible(panel, () => leave());
  });
  return (
    <div ref={panel} class="menu folder-pick" role="dialog">
      {props.head}
      <div class="mlabel">No server answers. Pair one to add a workspace on it.</div>
      <div class="sep" />
      <div class="pick-foot">
        <span>esc cancels</span>
        <button type="button" class="btn live" onClick={() => leave(commands.pairServer.run)}>
          {commands.pairServer.label}
        </button>
      </div>
    </div>
  );
}
