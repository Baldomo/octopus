import { FitAddon } from "@xterm/addon-fit";
import { WebglAddon } from "@xterm/addon-webgl";
import { type ITheme, Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { createEffect, createMemo, createSignal, For, on, onCleanup, onMount, Show } from "solid-js";
import { backend, Channel } from "./backend";
import { AGENT_PANEL, type Panel, type PtyId, type TaskRef, type TaskView } from "./generated";
import { Glyph } from "./Glyph";
import { commands, hint } from "./keys";
import { act, me, model, report, serverView, ui } from "./state";
import { hex, px, styleOf, token } from "./tokens";

let theme: ITheme | undefined;
function terminalTheme(): ITheme {
  const color = (name: string) => hex(token(name));
  theme ??= {
    background: color("--term"),
    foreground: hex(styleOf(["term"], "color")),
    cursor: color("--text"),
    selectionBackground: color("--line2"),
    black: color("--raised"),
    red: color("--err"),
    green: color("--working"),
    yellow: color("--needs"),
    blue: color("--info"),
    magenta: color("--violet"),
    cyan: color("--info"),
    white: color("--dim"),
    brightBlack: color("--faint"),
    brightRed: color("--err"),
    brightGreen: color("--working"),
    brightYellow: color("--needs"),
    brightBlue: color("--info"),
    brightMagenta: color("--violet"),
    brightCyan: color("--info"),
    brightWhite: color("--text"),
  };
  return theme;
}

const SCROLLBACK_LINES = 10_000;
/** A dropped stream retries after this, by which time a server that went away reads as unreachable. */
const REATTACH_MS = 500;

/**
 * A terminal keeps its screen while its task stays mounted, and holds a data stream only while on screen and its
 * server is reachable. Keys on a free lock go straight to the server, which gives the lock to the first writer.
 */
function Term(props: { at: TaskRef; panel: Panel; visible: boolean }) {
  let host!: HTMLDivElement;
  const mine = () => props.panel.lock_holder === me();
  const writable = () => props.panel.running && (props.panel.lock_holder === null || mine());
  const xterm = new Terminal({
    fontFamily: token("--mono-terminal"),
    fontSize: px("--fs-mono"),
    theme: terminalTheme(),
    cursorBlink: true,
    scrollback: SCROLLBACK_LINES,
  });
  const fit = new FitAddon();
  xterm.loadAddon(fit);
  let attachment: number | null = null;
  let generation = 0;
  /** Counts streams that ended under a running PTY, each of which calls for a fresh attachment. */
  const [drops, setDrops] = createSignal(0);

  /** The server sends the scrollback first, so the screen resets on the first chunk; an empty chunk ends the stream. */
  const attach = async () => {
    const current = ++generation;
    const output = new Channel<ArrayBuffer>();
    let replaying = true;
    let ended = false;
    output.onmessage = (bytes) => {
      if (current !== generation) return;
      if (bytes.byteLength === 0) {
        ended = true;
        attachment = null;
        if (props.panel.running) setTimeout(() => setDrops((count) => count + 1), REATTACH_MS);
        return;
      }
      if (replaying) {
        xterm.reset();
        replaying = false;
      }
      xterm.write(new Uint8Array(bytes));
    };
    const opened = await backend.termAttach(props.at, props.panel.id, output);
    if (current === generation && !ended) attachment = opened;
    else if (current !== generation) act(backend.termDetach(opened));
  };
  const detach = () => {
    generation++;
    if (attachment !== null) act(backend.termDetach(attachment));
    attachment = null;
  };
  const resize = () => act(backend.termResize(props.at, props.panel.id, xterm.cols, xterm.rows));

  xterm.onData((keys) => {
    if (attachment !== null && writable()) act(backend.termInput(attachment, keys));
  });
  xterm.onResize(() => {
    if (mine()) resize();
  });

  onMount(() => {
    xterm.open(host);
    try {
      const webgl = new WebglAddon();
      webgl.onContextLoss(() => webgl.dispose());
      xterm.loadAddon(webgl);
    } catch (error) {
      report(`WebGL is unavailable, so terminals draw through the DOM: ${error}`);
    }
    const observer = new ResizeObserver(() => {
      if (props.visible) fit.fit();
    });
    observer.observe(host);
    onCleanup(() => observer.disconnect());
  });
  const streaming = createMemo(() => props.visible && serverView(props.at.server)?.reachable === true);
  createEffect(
    on([streaming, drops], ([stream]) => {
      if (!stream) return;
      fit.fit();
      xterm.focus();
      attach().catch(report);
      onCleanup(detach);
    }),
  );
  createEffect(() => {
    xterm.options.disableStdin = !writable();
  });
  createEffect(on(mine, (holding) => holding && resize(), { defer: true }));
  onCleanup(() => {
    detach();
    xterm.dispose();
  });

  return (
    <div class="term-panel" style={{ display: props.visible ? "" : "none" }}>
      <div class="term">
        <div class="term-host" ref={host} />
      </div>
      <Show when={props.panel.lock_holder !== null && !mine()}>
        <div class="readonly">
          <span>read-only · held by {props.panel.lock_holder}</span>
          <span class="spacer" />
          <button class="btn live" onClick={() => act(backend.termTakeLock(props.at, props.panel.id))}>
            Take over
          </button>
        </div>
      </Show>
    </div>
  );
}

function Missing(props: { open: TaskView }) {
  const install = () => {
    const agent = props.open.task.agent;
    return agent.kind === "harness_not_installed" ? agent.install : null;
  };
  return (
    <div class="missing">
      <Show
        when={install() !== null}
        fallback={
          <div>
            {props.open.at.server} has no harness {props.open.task.harness}.
          </div>
        }
      >
        <div>
          {props.open.task.harness} is not installed on {props.open.at.server}.
        </div>
        <code>{install()}</code>
      </Show>
    </div>
  );
}

/** The agent's tab carries the task's glyph and names the harness it runs. */
function AgentTab(props: { open: TaskView; on: boolean; choose: () => void }) {
  return (
    <button class="tab agent" classList={{ on: props.on }} onClick={() => props.choose()}>
      <Glyph glyph={props.open.glyph} />
      {props.open.task.harness}
      <span class="kind">agent</span>
    </button>
  );
}

/** The agent tab comes first and is selected unless a shell tab is chosen. */
function Stack(props: { key: string }) {
  const task = () => model.tasks[props.key];
  const shown = () => ui.prefs.open === props.key && ui.prefs.view === "task";
  const [chosen, setChosen] = createSignal<PtyId | null>(null);
  const panels = () => task()?.task.panels ?? [];
  const agent = () => panels().find((panel) => panel.name === AGENT_PANEL);
  const shells = () => panels().filter((panel) => panel.name !== AGENT_PANEL);
  const selected = () => shells().find((panel) => panel.id === chosen()) ?? agent();
  const agentOn = () => selected() === agent();
  const missing = () => {
    const kind = task()?.task.agent.kind;
    return kind === "harness_unknown" || kind === "harness_not_installed";
  };
  return (
    <div class="pane stack" style={{ display: shown() ? "" : "none" }}>
      <Show when={task()}>
        {(open) => (
          <>
            <div class="tabs">
              <AgentTab open={open()} on={agentOn()} choose={() => setChosen(null)} />
              <For each={shells()}>
                {(panel) => (
                  <button
                    class="tab"
                    classList={{ on: selected()?.id === panel.id }}
                    onClick={() => setChosen(panel.id)}
                  >
                    {panel.name}
                    <span
                      class="close"
                      onClick={(event) => {
                        event.stopPropagation();
                        act(backend.termClose(open().at, panel.id));
                      }}
                    >
                      ×
                    </span>
                  </button>
                )}
              </For>
              <button class="tab plus" onClick={() => act(backend.termOpen(open().at).then(setChosen))}>
                +
              </button>
              <span class="spacer" />
              <Show when={selected()}>
                {(panel) => (
                  <span class="lock">{panel().lock_holder === null ? "lock free" : `✎ ${panel().lock_holder}`}</span>
                )}
              </Show>
            </div>
            <Show when={agentOn() && agent() === undefined}>
              <Show when={missing()} fallback={<div class="term" />}>
                <Missing open={open()} />
              </Show>
            </Show>
            <For each={panels().map((panel) => panel.id)}>
              {(id) => (
                <Show when={panels().find((panel) => panel.id === id)}>
                  {(panel) => <Term at={open().at} panel={panel()} visible={shown() && selected()?.id === id} />}
                </Show>
              )}
            </For>
          </>
        )}
      </Show>
    </div>
  );
}

/** The right half: one stack per recently opened task, the open one shown. */
export function TerminalStacks() {
  return (
    <>
      <For each={ui.recent}>{(key) => <Stack key={key} />}</For>
      <Show when={ui.prefs.open === null}>
        <div class="pane stack">
          <div class="empty">Open a task from the sidebar ({hint(commands.sidebar)}).</div>
        </div>
      </Show>
    </>
  );
}
