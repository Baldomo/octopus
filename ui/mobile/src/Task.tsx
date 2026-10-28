import { createEffect, createSignal, For, onCleanup, onMount, Show } from "solid-js";
import { backend } from "@octopus/shared/backend.ts";
import { AGENT_PANEL, type Panel, type PtyId, type TaskRef, type TaskView } from "@octopus/shared/generated.ts";
import { Glyph } from "@octopus/shared/Glyph.tsx";
import { act, me, model, scale, serverLabel, serverView, ui } from "@octopus/shared/state.ts";
import { Missing } from "@octopus/shared/Terminals.tsx";
import { terminalSession } from "@octopus/shared/terminalSession.ts";
import { px } from "@octopus/shared/tokens.ts";
import { micDenied, openKey, openTaskSheet, showTasks } from "./screen";

/** A finger that moves further than this is dragging, not tapping. */
const TAP_SLOP_PX = 8;
/** The corner map's width; its height follows the terminal's shape. */
const MAP_PX = 72;
/** The keyboard's text field always holds this, so its delete has something to delete even when the line is empty. */
const SENTINEL = " ";
const ESC = "\x1b";

/** A letter or one of `@[\]^_` as its control character; anything else as itself. */
const control = (char: string) =>
  /^[a-z@[\\\]^_]$/i.test(char) ? String.fromCharCode(char.toUpperCase().charCodeAt(0) & 0x1f) : char;

const ARROWS: [string, string][] = [
  ["←", "D"],
  ["↑", "A"],
  ["↓", "B"],
  ["→", "C"],
];

const reachable = (at: TaskRef) => serverView(at.server)?.reachable === true;

type Size = { width: number; height: number };

function observeSize(element: HTMLElement, set: (size: Size) => void) {
  const observer = new ResizeObserver(() => set({ width: element.clientWidth, height: element.clientHeight }));
  observer.observe(element);
  onCleanup(() => observer.disconnect());
}

/**
 * One terminal panel: the holder types into it and sizes it to the pane; everyone else sees it at the holder's size,
 * cropped to the pane, panned by dragging, with a map of the whole terminal in the corner.
 */
function PhoneTerm(props: { at: TaskRef; panel: Panel; visible: boolean; agent: boolean }) {
  let pane!: HTMLDivElement;
  let host!: HTMLDivElement;
  let sink!: HTMLTextAreaElement;
  const session = terminalSession({ at: props.at, panel: () => props.panel, shown: () => props.visible });
  const { xterm, mine } = session;
  xterm.options.disableStdin = true;
  const live = () => reachable(props.at);
  /** The part of the pane the terminal shows through, and the terminal's whole screen. */
  const [view, setView] = createSignal<Size>({ width: 0, height: 0 });
  const [screen, setScreen] = createSignal<Size>({ width: 0, height: 0 });
  const [pan, setPan] = createSignal({ x: 0, y: 0 });
  const [ctrl, setCtrl] = createSignal(false);
  const [compose, setCompose] = createSignal("");
  const cropped = () => !mine() && (screen().width > view().width || screen().height > view().height);

  /** Text the keyboard inserts, the first character as its control character while Ctrl is latched. */
  const typed = (text: string) => {
    if (text === "") return;
    if (ctrl()) {
      setCtrl(false);
      text = control(text.charAt(0)) + text.slice(1);
    }
    session.input(text.replaceAll("\n", "\r"));
  };
  const flush = () => {
    if (compose() !== "") session.input(compose());
    setCompose("");
  };
  /** A key of the key bar writes the compose line's text first. */
  const key = (keys: string) => {
    flush();
    session.input(keys);
  };
  const arrow = (code: string) => `${ESC}${xterm.modes.applicationCursorKeysMode ? "O" : "["}${code}`;
  /** Ctrl writes the compose line's text like every key, and its next character skips the line: the keyboard types
   * into the terminal while it is latched. */
  const latch = () => {
    if (ctrl()) return setCtrl(false);
    flush();
    setCtrl(true);
    sink.focus();
  };

  let sent = SENTINEL;
  let composing = false;
  const resetSink = () => {
    sink.value = sent = SENTINEL;
    sink.setSelectionRange(SENTINEL.length, SENTINEL.length);
  };
  /** Writes what changed in the field since the last input: a delete as Backspace, an insertion as itself. */
  const sinkInput = () => {
    const value = sink.value;
    let common = 0;
    while (common < sent.length && common < value.length && sent[common] === value[common]) common++;
    session.input("\x7f".repeat(sent.length - common));
    typed(value.slice(common));
    sent = value;
    if (!composing) resetSink();
  };

  /** Dragging pans a cropped terminal and scrolls the scrollback past its edge; a held terminal's drag scrolls. */
  let last: { x: number; y: number } | null = null;
  let travelled = 0;
  let unscrolled = 0;
  const scrollBy = (pixels: number) => {
    const cell = screen().height / xterm.rows;
    if (!(cell > 0)) return;
    unscrolled += pixels;
    const lines = Math.trunc(unscrolled / cell);
    if (lines !== 0) {
      xterm.scrollLines(lines);
      unscrolled -= lines * cell;
    }
  };
  const drag = (event: TouchEvent) => {
    const touch = event.touches[0];
    if (last === null || touch === undefined) return;
    event.preventDefault();
    const [dx, dy] = [touch.clientX - last.x, touch.clientY - last.y];
    last = { x: touch.clientX, y: touch.clientY };
    travelled += Math.hypot(dx, dy);
    if (!cropped()) return scrollBy(-dy);
    const most = { x: screen().width - view().width, y: screen().height - view().height };
    const wanted = { x: pan().x - dx, y: pan().y - dy };
    const y = Math.min(Math.max(wanted.y, 0), Math.max(most.y, 0));
    setPan({ x: Math.min(Math.max(wanted.x, 0), Math.max(most.x, 0)), y });
    scrollBy(wanted.y - y);
  };

  onMount(() => {
    session.open(host);
    observeSize(host, (size) => {
      setView(size);
      if (session.shown() && mine()) session.fit();
    });
    observeSize(xterm.element!.querySelector<HTMLElement>(".xterm-screen")!, setScreen);
    pane.addEventListener("touchmove", drag, { passive: false });
    resetSink();
  });
  createEffect(() => {
    xterm.options.fontSize = px("--fs-mono") * scale(ui.prefs);
    if (session.shown() && mine()) session.fit();
  });
  createEffect(() => {
    if (xterm.element) xterm.element.style.transform = `translate(${-pan().x}px, ${-pan().y}px)`;
  });
  createEffect(() => {
    if (!cropped()) setPan({ x: 0, y: 0 });
  });

  const map = () => {
    const whole = screen();
    const ratio = MAP_PX / whole.width;
    const seen = view();
    return {
      box: { width: `${MAP_PX}px`, height: `${whole.height * ratio}px` },
      seen: {
        left: `${pan().x * ratio}px`,
        top: `${pan().y * ratio}px`,
        width: `${Math.min(seen.width, whole.width) * ratio}px`,
        height: `${Math.min(seen.height, whole.height) * ratio}px`,
      },
    };
  };

  return (
    <div class="mterm" style={{ display: props.visible ? "" : "none" }}>
      <div class="zoom-reset" classList={{ stale: !live() }}>
        <div
          ref={pane}
          class="term-pane"
          onTouchStart={(event) => {
            const touch = event.touches[0];
            last = touch === undefined ? null : { x: touch.clientX, y: touch.clientY };
            travelled = 0;
          }}
          onTouchEnd={(event) => {
            last = null;
            if (travelled >= TAP_SLOP_PX || !mine() || !live()) return;
            // The click a tap synthesizes would land on the pane and take the focus back from the keyboard.
            event.preventDefault();
            sink.focus();
          }}
        >
          <div ref={host} class="term-host" />
          <Show when={cropped()}>
            <div class="map">
              <div class="map-whole" style={map().box}>
                <i style={map().seen} />
              </div>
              <span>
                {props.panel.cols}×{props.panel.rows}
              </span>
            </div>
          </Show>
        </div>
      </div>
      <textarea
        ref={sink}
        class="sink"
        autocapitalize="off"
        autocomplete="off"
        autocorrect="off"
        spellcheck={false}
        enterkeyhint="enter"
        onInput={sinkInput}
        onCompositionStart={() => (composing = true)}
        onCompositionEnd={() => {
          composing = false;
          resetSink();
        }}
        onKeyDown={(event) => {
          if (event.key !== "Enter") return;
          event.preventDefault();
          session.input("\r");
        }}
      />
      <div class="foot">
        <Show when={live()} fallback={<div class="foot-line">reconnecting to {serverLabel(props.at.server)}…</div>}>
          <Show
            when={mine()}
            fallback={
              <Show when={props.panel.running && props.panel.lock_holder !== null}>
                <div class="foot-line readonly">
                  <span>read-only · held by {props.panel.lock_holder}</span>
                  <span class="spacer" />
                  <button class="btn live" onClick={session.take}>
                    Take over
                  </button>
                </div>
              </Show>
            }
          >
            <form
              class="compose"
              onSubmit={(event) => {
                event.preventDefault();
                session.input(`${compose()}\r`);
                setCompose("");
              }}
            >
              <input
                value={compose()}
                placeholder={props.agent ? "Message the agent" : "Type a command"}
                autocapitalize="off"
                autocorrect="off"
                spellcheck={false}
                enterkeyhint="send"
                onFocus={() => setCtrl(false)}
                onInput={(event) => setCompose(event.currentTarget.value)}
              />
              <button type="submit" class="send" aria-label="Send">
                ↵
              </button>
            </form>
            <div class="keys" onPointerDown={(event) => event.preventDefault()}>
              <button onClick={() => key(ESC)}>Esc</button>
              <button onClick={() => key("\t")}>Tab</button>
              <button classList={{ on: ctrl() }} onClick={latch}>
                Ctrl
              </button>
              <For each={ARROWS}>{([label, code]) => <button onClick={() => key(arrow(code))}>{label}</button>}</For>
              <button onClick={() => key(`${ESC}\r`)}>⇧↵</button>
            </div>
          </Show>
        </Show>
      </div>
    </div>
  );
}

/**
 * "✎ you" while this phone holds the selected terminal, "✎ <client>" while another does, then whether it listens, and
 * whether this phone's microphone is denied to it.
 */
function lockMark(panel: Panel) {
  const mine = panel.lock_holder === me();
  const holder = panel.lock_holder === null ? "" : `✎ ${mine ? "you" : panel.lock_holder}`;
  const listening = panel.listening ? " · listening" : "";
  const denied = panel.listening && mine && micDenied() ? " · mic denied" : "";
  return holder + listening + denied;
}

function TaskScreen(props: { open: TaskView }) {
  const [chosen, setChosen] = createSignal<PtyId | null>(null);
  const panels = () => props.open.task.panels;
  const agent = () => panels().find((panel) => panel.name === AGENT_PANEL);
  const shells = () => panels().filter((panel) => panel.name !== AGENT_PANEL);
  const selected = () => shells().find((panel) => panel.id === chosen()) ?? agent();
  const agentOn = () => selected() === agent();
  const missing = () => ["harness_unknown", "harness_not_installed"].includes(props.open.task.agent.kind);
  const live = () => reachable(props.open.at);
  return (
    <main class="task-screen">
      <header class="task-head">
        <button class="back" aria-label="Tasks" onClick={showTasks}>
          ‹
        </button>
        <div class="task-title">
          <b>{props.open.task.title}</b>
          <span>
            {props.open.section} · {serverLabel(props.open.at.server)}
          </span>
        </div>
        <button class="pill" classList={{ ended: !props.open.live }} onClick={() => openTaskSheet(props.open.key)}>
          {props.open.task.state}
        </button>
      </header>
      <div class="tabs">
        <button class="tab agent" classList={{ on: agentOn() }} onClick={() => setChosen(null)}>
          <Glyph glyph={props.open.glyph} />
          {props.open.task.harness}
          <span class="kind">agent</span>
        </button>
        <For each={shells()}>
          {(panel) => (
            <button class="tab" classList={{ on: selected()?.id === panel.id }} onClick={() => setChosen(panel.id)}>
              {panel.name}
              <span
                class="close"
                onClick={(event) => {
                  event.stopPropagation();
                  if (live()) act(backend.termClose(props.open.at, panel.id));
                }}
              >
                ×
              </span>
            </button>
          )}
        </For>
        <button class="tab plus" onClick={() => live() && act(backend.termOpen(props.open.at).then(setChosen))}>
          +
        </button>
        <span class="spacer" />
        <Show when={selected()}>{(panel) => <span class="lock">{lockMark(panel())}</span>}</Show>
      </div>
      <Show when={agentOn() && agent() === undefined}>
        <Show when={missing()} fallback={<div class="mterm" />}>
          <Missing open={props.open} />
        </Show>
      </Show>
      <For each={panels().map((panel) => panel.id)}>
        {(id) => (
          <Show when={panels().find((panel) => panel.id === id)}>
            {(panel) => (
              <PhoneTerm
                at={props.open.at}
                panel={panel()}
                visible={selected()?.id === id}
                agent={panel().name === AGENT_PANEL}
              />
            )}
          </Show>
        )}
      </For>
    </main>
  );
}

/** The open task, while the model still lists it; another task opened starts its screen afresh. */
export function Task() {
  return (
    <Show when={openKey()} keyed>
      {(key) => <Show when={model.tasks[key]}>{(view) => <TaskScreen open={view()} />}</Show>}
    </Show>
  );
}

export const taskShown = () => {
  const key = openKey();
  return key !== null && model.tasks[key] !== undefined;
};
