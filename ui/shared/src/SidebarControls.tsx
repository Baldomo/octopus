import { createResource, createSignal, For, Show } from "solid-js";
import { backend } from "./backend";
import type { Swatch } from "./generated";
import { anyMenuOpen } from "./Menu";
import { model, report, serverLabel, servers } from "./state";
import { pickSwatch, sameSwatch, SWATCHES } from "./swatches";
import { swatchColor } from "./tokens";
import { toast } from "./Toast";

type Renamed = { kind: "server" | "section" | "task"; key: string };
const [renaming, setRenaming] = createSignal<Renamed | null>(null);
export const startRename = (kind: Renamed["kind"], key: string) => setRenaming({ kind, key });
export const isRenaming = (kind: Renamed["kind"], key: string) => renaming()?.kind === kind && renaming()?.key === key;

/** Edits a name in place: Enter applies a changed, non-empty name, and Esc or leaving the field cancels. */
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
        if (name !== "" && name !== props.value) props.apply(name).catch(report);
      }}
      onBlur={() => setRenaming(null)}
    />
  );
}

/** The color matrix, `current` ringed and `auto` dotted; picking `auto` picks null, the auto color. */
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

/** The sidebar holds the keyboard while one of its menus or editors is open. */
export const holdsKeyboard = () => anyMenuOpen() || renaming() !== null || adding();

export function AddWorkspace() {
  return (
    <Show
      when={adding()}
      fallback={
        <button class="addws" onClick={() => setAdding(true)}>
          <span>+</span>workspace
        </button>
      }
    >
      <WorkspaceCard />
    </Show>
  );
}

function WorkspaceCard() {
  const [server, setServer] = createSignal(servers().find((server) => server.reachable)?.name ?? "");
  const [name, setName] = createSignal("");
  const [root, setRoot] = createSignal("");
  /** Null until the user picks a color other than auto. */
  const [picked, setPicked] = createSignal<Swatch | null>(null);
  const [auto] = createResource(() => name().trim(), backend.autoColor);
  const color = () => {
    const swatch = picked() ?? auto();
    return swatch && swatchColor(swatch);
  };
  const [error, setError] = createSignal<string | null>(null);
  const [sending, setSending] = createSignal(false);
  const submit = () => {
    if (sending()) return;
    const [chosen, named, rooted] = [server(), name().trim(), root().trim()];
    const [swatch, resolved] = [picked(), auto.state === "ready" ? auto() : undefined];
    const stored = swatch && pickSwatch(swatch, resolved);
    const needs = [
      chosen === "" && "a server",
      named === "" && "name",
      !rooted.startsWith("/") && "an absolute root",
    ].filter((field) => field !== false);
    if (needs.length > 0) return setError(`Needs ${needs.join(" and ")}.`);
    if (model.lists.workspaces.some((workspace) => workspace.server === chosen && workspace.name === named)) {
      return setError(`${serverLabel(chosen)} already has ${named}.`);
    }
    setSending(true);
    backend
      .workspaceAdd(chosen, named, rooted, stored)
      .then(
        () => {
          setAdding(false);
          toast(
            <>
              Added <b>{named}</b> on {chosen}
            </>,
          );
        },
        (refused) => setError(String(refused)),
      )
      .finally(() => setSending(false));
  };
  const keys = (event: KeyboardEvent) => {
    if (event.key === "Enter" && event.target instanceof HTMLInputElement) submit();
    if (event.key === "Escape") setAdding(false);
  };
  return (
    <div class="card" style={{ "--ws": color() }} onKeyDown={keys}>
      <div class="ttl">
        <i />
        Add a workspace
      </div>
      <div class="field">
        <span>Server</span>
        <div class="pick">
          <For each={servers()}>
            {(option) => (
              <button
                class="host"
                classList={{ on: server() === option.name }}
                disabled={!option.reachable}
                title={option.reachable ? undefined : "unreachable"}
                onClick={() => setServer(option.name)}
              >
                <span class="dot" classList={{ off: !option.reachable }} />
                <span class="name">{option.label}</span>
              </button>
            )}
          </For>
        </div>
      </div>
      <label class="field">
        <span>Name</span>
        <input
          ref={(input) => requestAnimationFrame(() => input.focus())}
          spellcheck={false}
          placeholder="octopus"
          value={name()}
          onInput={(event) => setName(event.currentTarget.value)}
        />
      </label>
      <label class="field">
        <span>Root</span>
        <input
          class="mono"
          spellcheck={false}
          placeholder="/Users/you/src/octopus"
          value={root()}
          onInput={(event) => setRoot(event.currentTarget.value)}
        />
      </label>
      <Show when={auto()}>
        {(auto) => (
          <div class="field">
            <span>Color</span>
            <Matrix current={picked() ?? auto()} auto={auto()} pick={setPicked} />
          </div>
        )}
      </Show>
      <Show when={error()}>{(message) => <div class="err">{message()}</div>}</Show>
      <div class="row">
        <button class="btn" onClick={() => setAdding(false)}>
          Cancel
        </button>
        <button class="btn go" onClick={submit}>
          Add
        </button>
      </div>
    </div>
  );
}
