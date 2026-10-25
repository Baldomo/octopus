import { batch, createResource, createSignal, For, onCleanup, Show, type JSX } from "solid-js";
import { backend } from "./backend";
import {
  TASK_STATES,
  type Column,
  type Offer,
  type PairedClient,
  type Row,
  type Section,
  type TaskState,
} from "./generated";
import { mac } from "./keys";
import { anyMenuOpen, closeMenu, Head, Item, Sep } from "./Menu";
import { act, me, model, report, serverView, servers, setNewTaskSection, showBoard, showModal } from "./state";

const TOAST_MS = 1800;

const [said, setSaid] = createSignal<{ message: JSX.Element; nth: number } | null>(null);

/** Confirms what an action did, briefly, at the foot of the sidebar. */
export function toast(message: JSX.Element) {
  const nth = (said()?.nth ?? 0) + 1;
  setSaid({ message, nth });
  setTimeout(() => said()?.nth === nth && setSaid(null), TOAST_MS);
}

export const Toast = () => (
  <div class="toast" classList={{ on: said() !== null }}>
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

const clock = (at: number) => new Date(at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });

function ago(at: number, now: number) {
  const minutes = Math.floor((now - at) / 60_000);
  if (minutes < 1) return "just now";
  if (minutes < 60) return `${minutes} min ago`;
  const hours = Math.floor(minutes / 60);
  return hours < 24 ? `${hours} h ago` : `${Math.floor(hours / 24)} d ago`;
}

function ticking() {
  const [now, setNow] = createSignal(Date.now());
  const timer = setInterval(() => setNow(Date.now()), 1000);
  onCleanup(() => clearInterval(timer));
  return now;
}

const copyKey = mac ? "⌘C" : "Ctrl+C";

const grouped = (code: string) => code.match(/.{1,4}/g)?.join(" ") ?? code;

export function ServerMenu(props: { name: string }) {
  const server = () => serverView(props.name);
  const now = ticking();
  const [mode, setMode] = createSignal<"main" | "forget" | Offer>("main");
  const offer = () => {
    const shown = mode();
    return typeof shown === "string" ? null : shown;
  };
  const copyCode = (offered: Offer) => copy(offered.code, "the pairing code").then(closeMenu, report);
  const copyShortcut = (event: KeyboardEvent) => {
    const code = offer();
    if (code === null || event.key !== "c" || !(mac ? event.metaKey : event.ctrlKey)) return;
    event.preventDefault();
    copyCode(code);
  };
  document.addEventListener("keydown", copyShortcut);
  onCleanup(() => document.removeEventListener("keydown", copyShortcut));
  const [clients, { refetch }] = createResource(
    () => (server()?.reachable ? props.name : false),
    backend.serverClients,
  );
  const local = () => server()?.endpoint === null;
  const showCode = () => backend.serverOffer(props.name).then(setMode, report);
  const left = (offered: Offer) => {
    const seconds = Math.max(0, Math.ceil((offered.expires_at - now()) / 1000));
    return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
  };
  const status = (client: PairedClient) =>
    client.name === me()
      ? "this client"
      : client.connected
        ? "connected"
        : client.seen_at === null
          ? "never connected"
          : `seen ${ago(client.seen_at, now())}`;
  const revoke = (client: string) =>
    backend.serverRevoke(props.name, client).then(() => {
      refetch();
      toast(
        <>
          Revoked <b>{client}</b> on {props.name}
        </>,
      );
    }, report);
  const forget = () =>
    backend.serverForget(props.name).then(() => {
      closeMenu();
      toast(
        <>
          Forgot <b>{props.name}</b>
        </>,
      );
    }, report);

  return (
    <>
      <Head
        title={props.name}
        lines={[
          server()?.endpoint ?? "local socket",
          server()?.reachable
            ? `reachable · ${server()?.rtt_ms ?? "–"} ms`
            : `unreachable since ${clock(server()?.since ?? now())}`,
        ]}
      />
      <Show when={offer()}>
        {(offered) => (
          <>
            <div class="code">
              <div class="digits">{grouped(offered().code)}</div>
              <div class="exp">
                {left(offered()) === "0:00" ? "expired" : `one pairing · expires in ${left(offered())}`}
              </div>
            </div>
            <Item kbd={copyKey} run={() => copyCode(offered())}>
              Copy code
            </Item>
          </>
        )}
      </Show>
      <Show when={mode() === "forget"}>
        <div class="confirm">
          <span>
            Forget {props.name}? Its tasks leave this client's lists; they keep running on {props.name}.{" "}
            {local() ? "Adding it again takes octopus server add." : "Pairing again needs a new code."}
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
            <div class="client">
              <span class="mono">
                {!server()?.reachable
                  ? "unknown while unreachable"
                  : clients.error
                    ? String(clients.error)
                    : clients.loading
                      ? "listing…"
                      : "only this client, over the local socket"}
              </span>
            </div>
          }
        >
          <For each={clients()}>
            {(client) => (
              <div class="client">
                <span class="dot" classList={{ idle: !client.connected }} />
                {client.name}
                <span class="mono">{status(client)}</span>
                <Show when={client.name !== me()}>
                  <button class="mini" onClick={() => revoke(client.name)}>
                    Revoke
                  </button>
                </Show>
              </div>
            )}
          </For>
        </Show>
        <Sep />
        <Item run={closing(() => startRename("server", props.name))}>Rename…</Item>
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
  section.columns
    .filter((column) => column.live)
    .reduce((count, column) => count + column.rows.filter((row) => row.at.server === server).length, 0);

export function SectionMenu(props: { section: Section }) {
  const name = () => props.section.name;
  const workspaces = () => model.lists.workspaces.filter((workspace) => workspace.name === name());
  return (
    <>
      <Head title={name()} lines={workspaces().map((workspace) => `${workspace.server}  ${workspace.root}`)} />
      <Item run={closing(() => newTaskIn(name()))}>New task here</Item>
      <Item run={closing(() => showBoard(name()))}>Open board</Item>
      <Item run={closing(() => startRename("section", name()))}>Rename…</Item>
      <Sep />
      <For each={workspaces()}>
        {(workspace) => {
          const live = () => liveOn(props.section, workspace.server);
          const why = () =>
            !serverView(workspace.server)?.reachable ? "unreachable" : live() > 0 ? `${live()} live` : undefined;
          const remove = () =>
            backend.workspaceRemove(workspace.server, workspace.workspace).then(
              () =>
                toast(
                  <>
                    Removed <b>{workspace.name}</b> from {workspace.server} · branches kept
                  </>,
                ),
              report,
            );
          return (
            <Item danger why={why()} run={closing(remove)}>
              Remove from {workspace.server}
            </Item>
          );
        }}
      </For>
    </>
  );
}

export function TaskMenu(props: { row: Row; column: Column }) {
  const move = (state: TaskState) =>
    closing(() => {
      if (state === props.column.state) return;
      act(backend.taskMove(props.row.at, state));
      toast(
        <>
          <b>{props.row.title}</b> → {state}
        </>,
      );
    });
  return (
    <>
      <Head title={props.row.title} lines={[`${props.row.at.task} · ${props.row.at.server}`]} />
      <Item kbd="F2" run={closing(() => startRename("task", props.row.key))}>
        Rename…
      </Item>
      <Sep />
      <For each={TASK_STATES}>
        {(state) => (
          <Item check={state === props.column.state} run={move(state)}>
            {state}
          </Item>
        )}
      </For>
      <Sep />
      <Item run={() => copy(props.row.at.task, <b>{props.row.at.task}</b>).then(closeMenu, report)}>Copy task id</Item>
      <Item
        why={props.column.live ? "end it first" : undefined}
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

/** A root on any server's machine: a Unix path, a Windows drive path, or a UNC path. */
const absolute = (root: string) => /^(\/|[A-Za-z]:[\\/]|\\\\)/.test(root);

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
  const [error, setError] = createSignal<string | null>(null);
  const [sending, setSending] = createSignal(false);
  const submit = () => {
    if (sending()) return;
    const [chosen, named, rooted] = [server(), name().trim(), root().trim()];
    const needs = [chosen === "" && "a server", named === "" && "name", !absolute(rooted) && "an absolute root"].filter(
      (field) => field !== false,
    );
    if (needs.length > 0) return setError(`Needs ${needs.join(" and ")}.`);
    if (model.lists.workspaces.some((workspace) => workspace.server === chosen && workspace.name === named)) {
      return setError(`${chosen} already has ${named}.`);
    }
    setSending(true);
    backend
      .workspaceAdd(chosen, named, rooted)
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
    <div class="card" onKeyDown={keys}>
      <div class="ttl">Add a workspace</div>
      <div class="field">
        <span>Server</span>
        <div class="pick">
          <For each={servers()}>
            {(option) => (
              <button
                class="chip"
                classList={{ on: server() === option.name }}
                disabled={!option.reachable}
                title={option.reachable ? undefined : "unreachable"}
                onClick={() => setServer(option.name)}
              >
                <span class="dot" classList={{ off: !option.reachable }} />
                {option.name}
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
      <Show when={error()}>{(message) => <div class="err">{message()}</div>}</Show>
      <div class="row">
        <button class="btn" onClick={() => setAdding(false)}>
          Cancel
        </button>
        <button class="btn live" onClick={submit}>
          Add
        </button>
      </div>
    </div>
  );
}
