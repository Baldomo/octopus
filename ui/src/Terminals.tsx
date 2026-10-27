import { FitAddon } from "@xterm/addon-fit";
import { WebglAddon } from "@xterm/addon-webgl";
import { type ITheme, Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { createEffect, createMemo, createSignal, For, on, onCleanup, onMount, Show } from "solid-js";
import { backend, Channel, events } from "./backend";
import { AGENT_PANEL, type Panel, type PtyId, type TaskRef, type TaskView } from "./generated";
import { Glyph } from "./Glyph";
import { keepInView } from "./keepInView";
import { commands, hint, mac } from "./keys";
import { act, me, model, report, serverLabel, serverView, ui } from "./state";
import { colors, px, type Scheme, scheme, token } from "./tokens";

const themes: Partial<Record<Scheme, ITheme>> = {};
function terminalTheme(current: Scheme): ITheme {
  const color = colors(current);
  return (themes[current] ??= {
    background: color("--term"),
    foreground: color("--term-text"),
    cursor: color("--text"),
    selectionBackground: color("--line2"),
    overviewRulerBorder: "#00000000",
    black: color("--ansi-black"),
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
  });
}

/** Shift+Enter sends ESC CR, which agents read as a newline; on macOS ⌥←/⌥→ move by word as in Terminal.app. */
const CHORD_INPUT: Record<string, string> = {
  "shift+Enter": "\x1b\r",
  ...(mac && { "alt+ArrowLeft": "\x1bb", "alt+ArrowRight": "\x1bf" }),
};
/** Keys an input method consumes carry this code, including the one that commits the composition. */
const IME_KEY_CODE = 229;
const chord = (event: KeyboardEvent) =>
  [event.ctrlKey && "ctrl", event.altKey && "alt", event.metaKey && "meta", event.shiftKey && "shift", event.key]
    .filter(Boolean)
    .join("+");

const SCROLLBACK_LINES = 10_000;
/** A minimized or hidden window shows no terminal; one behind other windows still does. */
const [windowShown, setWindowShown] = createSignal(true);
events.windowShown(setWindowShown).catch(report);
/** A dropped stream retries after this, by which time a server that went away reads as unreachable. */
const REATTACH_MS = 500;

/**
 * A terminal keeps its screen while its task stays mounted, and holds a data stream only while on screen and its
 * window shows it and its server is reachable; the server reads that stream as this client showing the terminal.
 * A shown terminal with a free lock is claimed at the pane's size. The holder fits the PTY to its pane; every other
 * client draws it at the holder's size, cropped and scrollable.
 */
function Term(props: { at: TaskRef; panel: Panel; visible: boolean }) {
  let host!: HTMLDivElement;
  const mine = () => props.panel.lock_holder === me();
  const writable = () => props.panel.running && mine();
  const xterm = new Terminal({
    fontFamily: token("--mono-terminal"),
    fontSize: px("--fs-mono"),
    cursorBlink: true,
    scrollback: SCROLLBACK_LINES,
    overviewRuler: { width: px("--scrollbar-w") },
  });
  const fit = new FitAddon();
  xterm.loadAddon(fit);
  let attachment: number | null = null;
  /** Whether the server has this client's stream open, so a claim always reaches it showing the terminal. */
  const [attached, setAttached] = createSignal(false);
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
        setAttached(false);
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
    if (current === generation && !ended) {
      attachment = opened;
      setAttached(true);
    } else if (current !== generation) act(backend.termDetach(opened));
  };
  const detach = () => {
    generation++;
    if (attachment !== null) act(backend.termDetach(attachment));
    attachment = null;
    setAttached(false);
  };
  /** A drag of the window resizes once per frame, since every resize the server takes is published. */
  let resizing = 0;
  const resize = () => {
    cancelAnimationFrame(resizing);
    resizing = requestAnimationFrame(() => {
      if (mine()) act(backend.termResize(props.at, props.panel.id, xterm.cols, xterm.rows));
    });
  };
  const take = () => {
    const size = fit.proposeDimensions();
    if (size) act(backend.termTakeLock(props.at, props.panel.id, size.cols, size.rows));
    else report("the terminal's pane has no size to take the terminal at yet");
  };

  xterm.attachCustomKeyEventHandler((event) => {
    const input = CHORD_INPUT[chord(event)];
    if (input === undefined || event.keyCode === IME_KEY_CODE) return true;
    if (event.type === "keydown") {
      event.preventDefault();
      xterm.input(input);
    }
    return false;
  });
  xterm.onData((keys) => {
    if (attachment !== null) act(backend.termInput(attachment, keys));
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
      if (props.visible && mine()) fit.fit();
    });
    observer.observe(host);
    onCleanup(() => observer.disconnect());
  });
  const streaming = createMemo(() => props.visible && windowShown() && serverView(props.at.server)?.reachable === true);
  createEffect(
    on([streaming, drops], ([stream]) => {
      if (!stream) return;
      xterm.focus();
      attach().catch(report);
      onCleanup(detach);
    }),
  );
  createEffect(() => {
    xterm.options.disableStdin = !writable();
  });
  createEffect(() => {
    xterm.options.theme = terminalTheme(scheme());
  });
  createEffect(
    on([attached, () => props.panel.lock_holder === null && props.panel.running], ([showing, free]) => {
      if (!showing || !free) return;
      const size = fit.proposeDimensions();
      if (size) act(backend.termClaim(props.at, props.panel.id, size.cols, size.rows));
      else report("the terminal's pane has no size to claim the terminal at yet");
    }),
  );
  createEffect(
    on([mine, () => props.visible], ([holding, visible]) => {
      if (!visible || !holding) return;
      fit.fit();
      resize();
    }),
  );
  createEffect(
    on([mine, () => props.visible, () => props.panel.cols, () => props.panel.rows], ([holding, visible]) => {
      if (visible && !holding) xterm.resize(props.panel.cols, props.panel.rows);
    }),
  );
  onCleanup(() => {
    cancelAnimationFrame(resizing);
    detach();
    xterm.dispose();
  });

  return (
    <div class="term-panel" style={{ display: props.visible ? "" : "none" }}>
      <div class="term" classList={{ cropped: !mine() }}>
        <div class="term-host" ref={host} />
      </div>
      <Show when={props.panel.running && !mine()}>
        <div class="readonly">
          <span>
            {props.panel.lock_holder === null
              ? "read-only · lock free"
              : `read-only · held by ${props.panel.lock_holder}`}
          </span>
          <span class="spacer" />
          <button class="btn live" onClick={take}>
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
            {serverLabel(props.open.at.server)} has no harness {props.open.task.harness}.
          </div>
        }
      >
        <div>
          {props.open.task.harness} is not installed on {serverLabel(props.open.at.server)}.
        </div>
        <code>{install()}</code>
      </Show>
    </div>
  );
}

/** The agent's tab carries the task's glyph and names the harness it runs. */
function AgentTab(props: { open: TaskView; on: boolean; shown: boolean; choose: () => void }) {
  return (
    <button
      ref={keepInView(() => props.shown && props.on)}
      class="tab agent"
      classList={{ on: props.on }}
      onClick={() => props.choose()}
    >
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
              <AgentTab open={open()} on={agentOn()} shown={shown()} choose={() => setChosen(null)} />
              <For each={shells()}>
                {(panel) => {
                  const on = () => selected()?.id === panel.id;
                  return (
                    <button
                      ref={keepInView(() => shown() && on())}
                      class="tab"
                      classList={{ on: on() }}
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
                  );
                }}
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
