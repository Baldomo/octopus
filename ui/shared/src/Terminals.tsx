import { createEffect, For, on, onCleanup, onMount, Show } from "solid-js";
import { createStore } from "solid-js/store";
import { backend } from "./backend";
import { AGENT_PANEL, type PanelView, type PtyId, type TaskRef, type TaskView } from "./generated";
import { Glyph } from "./Glyph";
import { keepInView } from "./keepInView";
import { onFileDrag, pastesSafely, shellQuoted } from "./fileDrag";
import { EachPanel } from "./panels";
import { commands, hint, mac } from "./commands";
import { act, isLocal, model, serverLabel, ui } from "./state";
import { filesOf } from "./taskFiles";
import { toast } from "./Toast";
import { type LinkTarget, openLinks, type OpenUrl } from "./terminalLinks";
import { readOnlyLabel, terminalSession, windowShown } from "./terminalSession";

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

const [chosen, setChosen] = createStore<Record<string, PtyId | null>>({});

export const openShell = (at: TaskRef, key: string, directory: string) =>
  backend.termOpen(at, directory).then((panel) => setChosen(key, panel));

export type OpenLink = (task: TaskView, url: string, target: LinkTarget) => void;

/** Where files on this machine are on the task's server: there already when it runs here, else copied into the
 * checkout's root. */
async function onServer(task: TaskView, locals: string[]) {
  if (isLocal(task.at.server)) return locals;
  const copies = await filesOf(task).bringIn(locals, ".");
  return copies.map((copy) => `${task.task.directory}/${copy}`);
}

/** The holder fits the PTY to its pane; every other client draws it at the holder's size, cropped and scrollable. */
function Term(props: { task: TaskView; panel: PanelView; visible: boolean; resumable: boolean; openLink: OpenUrl }) {
  let host!: HTMLDivElement;
  let pane!: HTMLDivElement;
  const session = terminalSession({ at: props.task.at, panel: () => props.panel, shown: () => props.visible });
  const { xterm, mine, claimed } = session;

  xterm.attachCustomKeyEventHandler((event) => {
    const input = CHORD_INPUT[chord(event)];
    if (input === undefined || event.keyCode === IME_KEY_CODE) return true;
    if (event.type === "keydown") {
      event.preventDefault();
      xterm.input(input);
    }
    return false;
  });
  xterm.onData(session.input);
  openLinks(xterm, props.openLink);
  const pastable = () => {
    if (!props.panel.running) toast(`${props.panel.name} has ended`);
    else if (!mine()) toast("Take over the terminal to drop files into it");
    return props.panel.running && mine();
  };
  onFileDrag((drag, under) => {
    if (drag.kind !== "drop" || under === null || !pane.contains(under) || !pastable()) return;
    if (!drag.paths.every(pastesSafely))
      return toast("A path with a line break or a control character can't be pasted");
    const typed = onServer(props.task, drag.paths);
    act(typed.then((paths) => paths.length > 0 && pastable() && xterm.paste(paths.map(shellQuoted).join(" "))));
  });

  onMount(() => {
    session.open(host);
    const observer = new ResizeObserver(session.fit);
    observer.observe(host);
    onCleanup(() => observer.disconnect());
  });
  createEffect(
    on(
      () => session.shown() && windowShown(),
      (shown) => shown && xterm.focus(),
    ),
  );
  createEffect(() => {
    xterm.options.disableStdin = !(props.panel.running && mine());
  });

  return (
    <div class="term-panel" ref={pane} style={{ display: props.visible ? "" : "none" }}>
      <div class="term" classList={{ cropped: !claimed() }}>
        <div class="term-host" ref={host} />
      </div>
      <Show when={props.panel.running && !claimed()}>
        <div class="readonly">
          <span>{readOnlyLabel(props.panel)}</span>
          <span class="spacer" />
          <button class="btn live" onClick={session.take}>
            Take over
          </button>
        </div>
      </Show>
      <Show when={props.resumable}>
        <div class="readonly">
          <span>The agent has ended</span>
          <span class="spacer" />
          <button class="btn live" onClick={() => act(backend.taskFocus(props.task.at))}>
            Resume
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

function Stack(props: { key: string; openLink: OpenLink }) {
  const task = () => model.tasks[props.key];
  const shown = () => ui.prefs.open === props.key && ui.prefs.view === "task";
  const choose = (panel: PtyId | null) => setChosen(props.key, panel);
  const panels = () => task()?.panels ?? [];
  const agent = () => panels().find((panel) => panel.name === AGENT_PANEL);
  const shells = () => panels().filter((panel) => panel.name !== AGENT_PANEL);
  const selected = () => shells().find((panel) => panel.id === chosen[props.key]) ?? agent();
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
              <AgentTab open={open()} on={agentOn()} shown={shown()} choose={() => choose(null)} />
              <For each={shells()}>
                {(panel) => {
                  const on = () => selected()?.id === panel.id;
                  return (
                    <button
                      ref={keepInView(() => shown() && on())}
                      class="tab"
                      classList={{ on: on() }}
                      onClick={() => choose(panel.id)}
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
              <button class="tab plus" onClick={() => act(openShell(open().at, props.key, "."))}>
                +
              </button>
              <span class="spacer" />
              <Show when={selected()}>
                {(panel) => (
                  <span class="lock">
                    {panel().lock_holder === null ? "lock free" : `✎ ${panel().lock_holder?.name}`}
                    {panel().listening ? " · listening" : ""}
                  </span>
                )}
              </Show>
            </div>
            <Show when={agentOn() && agent() === undefined}>
              <Show when={missing()} fallback={<div class="term" />}>
                <Missing open={open()} />
              </Show>
            </Show>
            <EachPanel panels={panels()}>
              {(panel) => (
                <Term
                  task={open()}
                  panel={panel()}
                  visible={shown() && selected()?.id === panel().id}
                  resumable={open().live && panel().name === AGENT_PANEL && !panel().running}
                  openLink={(url, target) => props.openLink(open(), url, target)}
                />
              )}
            </EachPanel>
          </>
        )}
      </Show>
    </div>
  );
}

export function TerminalStacks(props: { openLink: OpenLink }) {
  return (
    <>
      <For each={ui.recent}>{(key) => <Stack key={key} openLink={props.openLink} />}</For>
      <Show when={ui.prefs.open === null}>
        <div class="pane stack">
          <div class="empty">Open a task from the sidebar ({hint(commands.sidebar)}).</div>
        </div>
      </Show>
    </>
  );
}
