import { batch, createResource, createSignal, For, onCleanup, Show, type JSX } from "solid-js";
import { backend } from "./backend";
import {
  HUES,
  LIVE_STATES,
  TASK_STATES,
  TONES,
  type Offer,
  type ShownOffer,
  type PairedClient,
  type Row,
  type Section,
  type Swatch,
  type TaskState,
} from "./generated";
import { commands, hint, mac } from "./commands";
import { anyMenuOpen, closeMenu, Head, Item, Sep } from "./Menu";
import {
  act,
  isLocal,
  myKey,
  model,
  report,
  serverLabel,
  serverView,
  servers,
  setNewTaskSection,
  showBoard,
  showModal,
  showSidebar,
} from "./state";
import { swatchColor } from "./tokens";

const TOAST_MS = 1800;

const [said, setSaid] = createSignal<{ message: JSX.Element; nth: number } | null>(null);

/** Confirms what an action did, briefly, at the foot of the sidebar or the board. */
export function toast(message: JSX.Element) {
  const nth = (said()?.nth ?? 0) + 1;
  setSaid({ message, nth });
  setTimeout(() => said()?.nth === nth && setSaid(null), TOAST_MS);
}

export const toasting = () => said() !== null;

export const Toast = (props: { ref?: (element: HTMLElement) => void }) => (
  <div ref={props.ref} class="toast" classList={{ on: toasting() }}>
    {said()?.message}
  </div>
);

type Renamed = { kind: "server" | "section" | "task"; key: string };
const [renaming, setRenaming] = createSignal<Renamed | null>(null);
export const startRename = (kind: Renamed["kind"], key: string) => setRenaming({ kind, key });
export const isRenaming = (kind: Renamed["kind"], key: string) => renaming()?.kind === kind && renaming()?.key === key;

/** In one batch, so a rename that replaces the menu keeps the keyboard in the sidebar throughout. */
const closing = (action: () => void) => () =>
  batch(() => {
    closeMenu();
    action();
  });

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

async function copy(text: string, what: JSX.Element) {
  await navigator.clipboard.writeText(text);
  toast(<>Copied {what}</>);
}

/** A time of day in the locale's clock, or on the 24-hour one a design asks for. */
export const clock = (at: number, hourCycle?: "h23") =>
  new Date(at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", hourCycle });

export function ago(at: number, now: number) {
  const minutes = Math.floor((now - at) / 60_000);
  if (minutes < 1) return "just now";
  if (minutes < 60) return `${minutes} min ago`;
  const hours = Math.floor(minutes / 60);
  return hours < 24 ? `${hours} h ago` : `${Math.floor(hours / 24)} d ago`;
}

export function ticking() {
  const [now, setNow] = createSignal(Date.now());
  const timer = setInterval(() => setNow(Date.now()), 1000);
  onCleanup(() => clearInterval(timer));
  return now;
}

const copyKey = mac ? "⌘C" : "Ctrl+C";

export const grouped = (code: string) => code.match(/.{1,4}/g)?.join(" ") ?? code;

export const inviteCode = (offered: Offer) => new URL(offered.invite).searchParams.get("code") ?? "";

/** Whether a server's paired client is this one: the server pins it by its key. */
export const isThisClient = (client: PairedClient) => client.key === myKey();

/** What a paired client's row says of it beside its name. */
export const clientStatus = (client: PairedClient, now: number) =>
  isThisClient(client)
    ? "this client"
    : client.connected
      ? "connected"
      : client.seen_at === null
        ? "never connected"
        : `seen ${ago(client.seen_at, now)}`;

/** The time left on an offer as m:ss, or "expired". */
export function expiry(offered: Offer, now: number) {
  const seconds = Math.max(0, Math.ceil((offered.expires_at - now) / 1000));
  return seconds === 0
    ? "expired"
    : `one pairing · expires in ${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
}

export function ServerMenu(props: { name: string }) {
  const server = () => serverView(props.name);
  const label = serverLabel(props.name);
  const now = ticking();
  const [mode, setMode] = createSignal<"main" | "forget" | ShownOffer>("main");
  const offer = () => {
    const shown = mode();
    return typeof shown === "string" ? null : shown;
  };
  const copyInvite = (offered: Offer) => copy(offered.invite, "the pairing invite").then(closeMenu, report);
  const copyShortcut = (event: KeyboardEvent) => {
    const offered = offer();
    if (offered === null || event.key !== "c" || !(mac ? event.metaKey : event.ctrlKey)) return;
    event.preventDefault();
    copyInvite(offered.offer);
  };
  document.addEventListener("keydown", copyShortcut);
  onCleanup(() => document.removeEventListener("keydown", copyShortcut));
  const [clients, { refetch }] = createResource(
    () => (server()?.reachable ? props.name : false),
    backend.serverClients,
  );
  const showCode = () => backend.serverOffer(props.name).then(setMode, report);
  const revoke = (client: string) =>
    backend.serverRevoke(props.name, client).then(() => {
      refetch();
      toast(
        <>
          Revoked <b>{client}</b> on {label}
        </>,
      );
    }, report);
  const forget = () =>
    backend.serverForget(props.name).then(() => {
      closeMenu();
      toast(
        <>
          Forgot <b>{label}</b>
        </>,
      );
    }, report);

  return (
    <>
      <Head
        title={props.name}
        lines={[
          server()?.relay ?? "local socket",
          server()?.reachable
            ? `reachable · ${server()?.rtt_ms ?? "–"} ms`
            : `unreachable since ${clock(server()?.since ?? now())}`,
        ]}
      />
      <Show when={offer()}>
        {(offered) => (
          <>
            <div class="code">
              <div class="qr" innerHTML={offered().qr} />
              <div class="digits">{grouped(inviteCode(offered().offer))}</div>
              <div class="exp">{expiry(offered().offer, now())}</div>
            </div>
            <Item kbd={copyKey} run={() => copyInvite(offered().offer)}>
              Copy invite
            </Item>
          </>
        )}
      </Show>
      <Show when={mode() === "forget"}>
        <div class="confirm">
          <span>
            Forget {label}? Its tasks leave this client's lists; they keep running on {label}.{" "}
            {isLocal(props.name) ? "It returns when the app next starts." : "Pairing again needs a new invite."}
          </span>
          <div class="row">
            <button class="btn" onClick={() => setMode("main")}>
              Cancel
            </button>
            <button class="btn danger" onClick={forget}>
              Forget
            </button>
          </div>
        </div>
      </Show>
      <Show when={mode() === "main"}>
        <Item run={showCode} why={server()?.reachable ? undefined : "unreachable"}>
          Show pairing code
        </Item>
        <Sep />
        <div class="mh label">
          <span>Paired clients</span>
        </div>
        <Show
          when={server()?.reachable && !clients.error && (clients() ?? []).length > 0}
          fallback={
            <div class="mh label">
              {!server()?.reachable
                ? "unknown while unreachable"
                : clients.error
                  ? String(clients.error)
                  : clients.loading
                    ? "listing…"
                    : "only this client, over the local socket"}
            </div>
          }
        >
          <For each={clients()}>
            {(client) => (
              <div class="client">
                <span class="dot" classList={{ idle: !client.connected }} />
                <span class="name">{client.name}</span>
                <span class="mono">{clientStatus(client, now())}</span>
                <Show when={!isThisClient(client)}>
                  <button class="mini" onClick={() => revoke(client.name)}>
                    Revoke
                  </button>
                </Show>
              </div>
            )}
          </For>
        </Show>
        <Sep />
        <Item
          run={closing(() => {
            showSidebar(true);
            startRename("server", props.name);
          })}
        >
          Rename…
        </Item>
        <Item danger run={() => setMode("forget")}>
          Forget server
        </Item>
      </Show>
    </>
  );
}

/** Opens the new-task form on the section's workspace. */
export function newTaskIn(section: string) {
  setNewTaskSection(section);
  showModal("new_task");
}

const liveOn = (section: Section, server: string) =>
  section.groups.reduce((count, group) => count + group.rows.filter((row) => row.at.server === server).length, 0);

export const isLive = (state: TaskState) => LIVE_STATES.some((live) => live === state);

const same = (a: Swatch, b: Swatch) => a.hue === b.hue && a.tone === b.tone;

const MATRIX: Swatch[] = TONES.flatMap((tone) => HUES.map((hue) => ({ hue, tone })));

/** The color matrix, `current` ringed and `auto` dotted; picking `auto` picks null, the auto color. */
function Matrix(props: { current: Swatch; auto: Swatch; pick: (swatch: Swatch | null) => void }) {
  return (
    <div class="matrix">
      <For each={MATRIX}>
        {(swatch) => (
          <button
            type="button"
            class="swatch"
            classList={{ on: same(swatch, props.current), auto: same(swatch, props.auto) }}
            style={{ "--c": swatchColor(swatch) }}
            title={`${swatch.hue} ${swatch.tone}${same(swatch, props.auto) ? " · auto" : ""}`}
            onClick={() => props.pick(same(swatch, props.auto) ? null : swatch)}
          />
        )}
      </For>
    </div>
  );
}

/** Finds its section by name on every model, so it follows changes made while it is open. */
export function SectionMenu(props: { name: string }) {
  const section = () => model.lists.sections.find((candidate) => candidate.name === props.name);
  const workspaces = () => model.lists.workspaces.filter((workspace) => workspace.name === props.name);
  return (
    <Show when={section()}>
      {(section) => (
        <>
          <Head
            title={props.name}
            color={swatchColor(section().swatch)}
            lines={workspaces().map((workspace) => `${serverLabel(workspace.server)}  ${workspace.root}`)}
          />
          <Item kbd={hint(commands.newTask)} run={closing(() => newTaskIn(props.name))}>
            New task here
          </Item>
          <Item run={closing(() => showBoard(props.name))}>Open board</Item>
          <Item run={closing(() => startRename("section", props.name))}>Rename…</Item>
          <div class="mlabel">Color</div>
          <Matrix
            current={section().swatch}
            auto={section().auto}
            pick={(swatch) => backend.sectionColor(props.name, swatch).catch(report)}
          />
          <Sep />
          <For each={workspaces()}>
            {(workspace) => {
              const live = () => liveOn(section(), workspace.server);
              const why = () =>
                !serverView(workspace.server)?.reachable ? "unreachable" : live() > 0 ? `${live()} live` : undefined;
              const remove = () =>
                backend.workspaceRemove(workspace.server, workspace.workspace).then(
                  () =>
                    toast(
                      <>
                        Removed <b>{workspace.name}</b> from {serverLabel(workspace.server)} · branches kept
                      </>,
                    ),
                  report,
                );
              return (
                <Item danger why={why()} run={closing(remove)}>
                  <span class="name">Remove from {serverLabel(workspace.server)}</span>
                </Item>
              );
            }}
          </For>
        </>
      )}
    </Show>
  );
}

export function TaskMenu(props: { row: Row }) {
  const move = (state: TaskState) =>
    closing(() => {
      if (state === props.row.state) return;
      act(backend.taskMove(props.row.at, state));
      toast(
        <>
          <b>{props.row.title}</b> → {state}
        </>,
      );
    });
  return (
    <>
      <Head title={props.row.title} lines={[`${props.row.at.task} · ${serverLabel(props.row.at.server)}`]} />
      <Item kbd="F2" run={closing(() => startRename("task", props.row.key))}>
        Rename…
      </Item>
      <Sep />
      <For each={TASK_STATES}>
        {(state) => (
          <Item check={state === props.row.state} run={move(state)}>
            {state}
          </Item>
        )}
      </For>
      <Sep />
      <Item run={() => copy(props.row.at.task, <b>{props.row.at.task}</b>).then(closeMenu, report)}>Copy task id</Item>
      <Item
        why={isLive(props.row.state) ? "end it first" : undefined}
        run={closing(() => act(backend.taskArchive(props.row.at)))}
      >
        Archive
      </Item>
    </>
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
    const stored = swatch !== null && resolved !== undefined && same(swatch, resolved) ? null : swatch;
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
