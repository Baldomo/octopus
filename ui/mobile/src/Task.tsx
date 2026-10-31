import { createEffect, createSignal, For, Match, on, onCleanup, onMount, Show, Switch } from "solid-js";
import { backend } from "@octopus/shared/backend.ts";
import { AGENT_PANEL, type Panel, type PtyId, type TaskRef, type TaskView } from "@octopus/shared/generated.ts";
import { ago, clock, ticking } from "@octopus/shared/SidebarControls.tsx";
import { act, me, model, report, scale, serverLabel, serverView, ui } from "@octopus/shared/state.ts";
import { cellSize, type Size, terminalSession } from "@octopus/shared/terminalSession.ts";
import { colors, scheme } from "@octopus/shared/tokens.ts";
import { feel } from "./feel";
import { animateList } from "./flip";
import { Icon } from "./Icon";
import { onPainted } from "./motion";
import { back, micDenied, openSheet } from "./nav";
import { Circle, Gap, Glyph, Id, Para, sectionColor, ServerDot, Slot } from "./parts";
import { phone } from "./phone";
import type { Point } from "./press";
import { type Scroll, scroller } from "./scroll";
import { TERMINAL_FONT } from "./symbols";
import { termSelection } from "./termSelect";
import { slidingThumb } from "./thumb";
import { notify } from "./Toast";
import { COMMIT_PX, owns, type Gesture } from "./touch";

/**
 * The keyboard's text field holds a block of spaces with its caret in the middle, so its delete always has something to
 * delete and a space-bar drag has room to move the caret every way.
 */
const PAD_COLS = 24;
const PAD_LINES = 3;
const SINK_COLS = 2 * PAD_COLS;
const SINK_ROWS = 2 * PAD_LINES + 1;
const PADDED = Array(SINK_ROWS).fill(" ".repeat(SINK_COLS)).join("\n");
const LINE_CHARS = SINK_COLS + 1;
const CENTER = PAD_LINES * LINE_CHARS + PAD_COLS;
const ESC = "\x1b";
/** The terminal sets 13pt Meslo on 20pt rows. */
const FONT_PX = 13;
const ROW_PX = 20;
/** How near a line boundary, in lines, the terminal counts as on it, so float error never rests it a line off. */
const ON_LINE = 1e-6;
/** How long the server keeps a dropped holder's lock, which the phone counts down while it reconnects. */
const LOCK_KEPT_MS = 10_000;

/** A letter or one of `@[\]^_` as its control character; anything else as itself. */
const control = (char: string) =>
  /^[a-z@[\\\]^_]$/i.test(char) ? String.fromCharCode(char.toUpperCase().charCodeAt(0) & 0x1f) : char;

const ARROWS: [string, string][] = [
  ["←", "D"],
  ["↑", "A"],
  ["↓", "B"],
  ["→", "C"],
];

function observeSize(element: HTMLElement, set: (size: Size) => void) {
  const observer = new ResizeObserver(() => set({ width: element.clientWidth, height: element.clientHeight }));
  observer.observe(element);
  onCleanup(() => observer.disconnect());
}

/** What the phone knows of the task's server: reachable, reconnecting while the lock it held is kept, or gone. */
type Reach = { is: "reachable" } | { is: "reconnecting"; left: number } | { is: "unreachable"; since: number };

function reachOf(at: TaskRef, panel: Panel | undefined, now: number): Reach {
  const server = serverView(at.server);
  if (server === undefined || server.reachable) return { is: "reachable" };
  const left = LOCK_KEPT_MS - (now - server.since);
  if (panel?.lock_holder === me() && left > 0) return { is: "reconnecting", left };
  return { is: "unreachable", since: server.since };
}

/**
 * One terminal panel. The holder types into it through the keyboard and the key bar and sizes it to the pane; everyone
 * else sees it at the holder's size, cropped to the pane at its last line, panned by dragging.
 */
function PhoneTerm(props: {
  at: TaskRef;
  panel: Panel;
  visible: boolean;
  reach: Reach;
  ended: boolean;
  resumable: boolean;
}) {
  let host!: HTMLDivElement;
  let pane!: HTMLDivElement;
  let sink!: HTMLTextAreaElement;
  /** The terminal opens once the screen is on its way in, and shows from then on. */
  const [opened, setOpened] = createSignal(false);
  const session = terminalSession({ at: props.at, panel: () => props.panel, shown: () => props.visible && opened() });
  const { xterm, mine } = session;
  xterm.options.disableStdin = true;
  xterm.options.cursorInactiveStyle = "block";
  xterm.options.fontFamily = TERMINAL_FONT;
  /** The phone shows its own thumbs, so the terminal's text runs to the pane's right edge. */
  xterm.options.overviewRuler = {};
  const live = () => props.reach.is === "reachable";
  const [view, setView] = createSignal<Size>({ width: 0, height: 0 });
  const [screen, setScreen] = createSignal<Size>({ width: 0, height: 0 });
  const [pan, setPan] = createSignal({ x: 0, y: 0 });
  const [ctrl, setCtrl] = createSignal(false);
  const cropped = () => !mine() && (screen().width > view().width || screen().height > view().height);
  const most = () => ({
    x: Math.max(screen().width - view().width, 0),
    y: Math.max(screen().height - view().height, 0),
  });

  const write = (keys: string) => {
    toLastLine();
    session.input(keys);
  };
  /** Text the keyboard inserts, the first character as its control character while Ctrl is latched. */
  const typed = (text: string) => {
    if (text === "") return;
    if (ctrl()) {
      setCtrl(false);
      text = control(text.charAt(0)) + text.slice(1);
    }
    write(text.replaceAll("\n", "\r"));
  };
  const arrow = (code: string) => `${ESC}${xterm.modes.applicationCursorKeysMode ? "O" : "["}${code}`;
  const arrows = (steps: number, backward: string, forward: string) =>
    arrow(steps < 0 ? backward : forward).repeat(Math.abs(steps));
  const latch = () => {
    feel("select");
    setCtrl(!ctrl());
    if (ctrl()) sink.focus();
  };
  const toggleKeyboard = () => {
    feel("tap");
    if (document.activeElement === sink) sink.blur();
    else sink.focus();
  };
  const send = (keys: string) => {
    feel("tap");
    write(keys);
  };
  /** Losing the lock to another client while showing it is worth a warning; taking it over, a success. */
  createEffect(
    on(
      () => props.visible && props.panel.running && props.panel.lock_holder,
      (holder, was) => {
        if (!holder || !was || holder === was) return;
        if (holder === me()) feel("success");
        else if (was === me()) feel("warning");
      },
      { defer: true },
    ),
  );

  let sent = PADDED;
  let restingCaret = CENTER;
  let composing = false;
  const resetSink = () => {
    sink.value = sent = PADDED;
    sink.setSelectionRange(CENTER, CENTER);
    restingCaret = CENTER;
  };
  /**
   * Writes what changed before the caret since the last input, the text after it staying as it was: a delete as
   * Backspace, an insertion as itself. A held delete that deletes by words writes Ctrl-W, since the field before the
   * caret is one run of blanks.
   */
  const sinkInput = (event: InputEvent) => {
    if (event.inputType === "deleteWordBackward") {
      write(control("w"));
      return resetSink();
    }
    const { value, selectionEnd: end } = sink;
    const sentBeforeCaret = sent.length - (value.length - end);
    let common = 0;
    while (common < sentBeforeCaret && common < end && sent[common] === value[common]) common++;
    write("\x7f".repeat(sentBeforeCaret - common));
    typed(value.slice(common, end));
    sent = value;
    if (!composing) resetSink();
  };
  /**
   * The space-bar drag moves the caret through the blank field, which moves the terminal's cursor as many cells. iOS
   * places the dragged caret at the point the finger has travelled to since the drag began, so the caret recenters
   * only once the keyboard next types or rises. A selection collapses to its end, so typing never replaces it.
   */
  const sinkCaretMoved = () => {
    const { selectionStart, selectionEnd: at } = sink;
    if (document.activeElement !== sink || composing || sink.value !== PADDED) return;
    if (selectionStart !== at) return sink.setSelectionRange(at, at);
    if (at === restingCaret) return;
    const lines = Math.floor(at / LINE_CHARS) - Math.floor(restingCaret / LINE_CHARS);
    const cols = (at % LINE_CHARS) - (restingCaret % LINE_CHARS);
    write(arrows(lines, "A", "B") + arrows(cols, "D", "C"));
    restingCaret = at;
  };
  /**
   * The terminal draws its cursor blinking while the keyboard types into it, and solid otherwise. xterm draws no cursor
   * until it has once been focused, so it opens drawn focused, then blurred.
   */
  const drawFocused = (focused: boolean) => xterm.textarea?.dispatchEvent(new FocusEvent(focused ? "focus" : "blur"));
  /** A tap on the cursor's line moves the cursor to the tapped cell, or to the end of the line's text past it. */
  const moveCursorTo = (point: Point) => {
    const buffer = xterm.buffer.active;
    const cell = selection.cellAt(point);
    if (cell?.row !== buffer.baseY + buffer.cursorY) return;
    const line = buffer.getLine(cell.row);
    let textEnd = line?.length ?? 0;
    while (textEnd > 0 && !line?.getCell(textEnd - 1)?.getChars()) textEnd--;
    const col = Math.min(cell.col, Math.max(buffer.cursorX, textEnd));
    if (col !== buffer.cursorX) write(arrows(col - buffer.cursorX, "D", "C"));
  };

  /**
   * The terminal scrolls as one surface from its first scrollback line down to the bottom of a cropped screen: the
   * scrollback by whole lines, with the part of a line between them and the crop as the pan. It comes to rest on a line.
   */
  const cell = () => screen().height / xterm.rows;
  const measured = () => cell() > 0;
  const scrolledLines = () => xterm.buffer.active.baseY;
  const downward: Scroll = {
    at: () => xterm.buffer.active.viewportY * cell() + pan().y,
    end: () => scrolledLines() * cell() + most().y,
    rest: (at) => (at < scrolledLines() * cell() ? Math.round(at / cell()) * cell() : at),
    apply: (at) => {
      const line = Math.min(Math.max(Math.floor(at / cell() + ON_LINE), 0), scrolledLines());
      xterm.scrollToLine(line);
      setPan({ x: pan().x, y: at - line * cell() });
    },
  };
  const down = scroller(downward);
  const toLastLine = () => {
    down.stop();
    if (measured()) downward.apply(downward.end());
  };
  const across = scroller({
    at: () => pan().x,
    end: () => most().x,
    rest: (at) => at,
    apply: (x) => setPan({ x, y: pan().y }),
  });
  /** Dragging scrolls the terminal, and pans a cropped one across too. */
  const drag: Gesture = ({ axis, at }) => {
    if (!measured() || (axis === "x" && !cropped())) return null;
    const x = cropped() ? across.grab() : null;
    const y = down.grab();
    scrolling = true;
    return {
      move: (to) => {
        x?.drag(at.x - to.x);
        y.drag(at.y - to.y);
      },
      end: (velocity) => {
        x?.release(-(velocity?.x ?? 0));
        y.release(-(velocity?.y ?? 0));
      },
    };
  };
  let from: Point | null = null;
  let scrolling = false;
  /** A touch the terminal's drag didn't scroll stopped any coast where it stood, so lifting it settles on a line. */
  const settle = () => {
    if (scrolling) return;
    down.settle();
    across.settle();
  };

  /** Rows of 20pt at 13pt: the line height that turns the font's own cell into a 20pt one, measured once drawn. */
  const fitRows = () => {
    const natural = cellSize(xterm).height / xterm.options.lineHeight!;
    if (natural > 0) xterm.options.lineHeight = (ROW_PX * scale(ui.prefs)) / natural;
  };

  let drawn: HTMLElement | undefined;
  const selection = termSelection({
    xterm,
    screen: () => drawn,
    size: screen,
    view,
    pan,
  });

  onMount(() => {
    owns(pane, drag);
    selection.attach(pane);
    resetSink();
    document.addEventListener("selectionchange", sinkCaretMoved);
    onCleanup(() => document.removeEventListener("selectionchange", sinkCaretMoved));
  });
  onPainted(() => {
    session.open(host);
    drawFocused(true);
    drawFocused(false);
    observeSize(host, (size) => {
      setView(size);
      session.fit();
    });
    drawn = xterm.element!.querySelector<HTMLElement>(".xterm-screen")!;
    observeSize(drawn, setScreen);
    setOpened(true);
  });
  /** A terminal mounted hidden measures no cell, so its rows fit again each time it shows. */
  createEffect(() => {
    if (!session.shown()) return;
    xterm.options.fontSize = FONT_PX * scale(ui.prefs);
    xterm.options.lineHeight = 1;
    requestAnimationFrame(() => {
      fitRows();
      session.fit();
    });
  });
  /** The terminal draws on the screen's one surface, its plain text dim while another client holds it. */
  createEffect(() => {
    const color = colors(scheme());
    xterm.options.theme = {
      ...xterm.options.theme,
      background: color("--bg"),
      foreground: color(mine() ? "--term-text" : "--dim"),
      cursor: color("--text"),
      cursorAccent: color("--bg"),
      selectionBackground: color("--select"),
    };
  });
  createEffect(() => {
    const { x, y } = pan();
    if (opened()) xterm.element!.style.transform = `translate(${-x}px, ${-y}px)`;
  });
  /** A cropped terminal opens at its last line. */
  createEffect(() => setPan(cropped() ? { x: 0, y: most().y } : { x: 0, y: 0 }));

  /** Two 3pt thumbs, one per axis the crop scrolls, sized and placed as the part on screen. */
  const thumbs = () => {
    const whole = screen();
    const seen = view();
    const across = Math.min(seen.width / whole.width, 1);
    const down = Math.min(seen.height / whole.height, 1);
    return {
      x: across < 1 && {
        left: `calc(var(--rail) + ${(pan().x / whole.width) * seen.width}px / var(--zoom, 1))`,
        width: `calc(${across * seen.width}px / var(--zoom, 1))`,
      },
      y: down < 1 && {
        top: `calc(var(--g2) + ${(pan().y / whole.height) * seen.height}px / var(--zoom, 1))`,
        height: `calc(${down * seen.height}px / var(--zoom, 1))`,
      },
    };
  };

  return (
    <div class="tterm fill" style={{ display: props.visible ? "" : "none" }}>
      <div
        ref={pane}
        class="term"
        classList={{ held: !mine(), stale: !live() }}
        onTouchStart={(event) => {
          const touch = event.touches[0];
          from = touch === undefined ? null : { x: touch.clientX, y: touch.clientY };
          scrolling = false;
          down.stop();
          across.stop();
        }}
        onTouchCancel={settle}
        onTouchEnd={(event) => {
          settle();
          if (event.defaultPrevented || selection.onControls(event)) return;
          const touch = event.changedTouches[0];
          if (!from || !touch || Math.hypot(touch.clientX - from.x, touch.clientY - from.y) >= COMMIT_PX) return;
          const point = { x: touch.clientX, y: touch.clientY };
          const tapped = selection.tap(point);
          if (!tapped && (!mine() || !live())) return;
          // The click a tap synthesizes would land on the pane and take the focus back from the keyboard.
          event.preventDefault();
          if (tapped) return;
          moveCursorTo(point);
          sink.focus();
        }}
      >
        <div class="zoom-reset">
          <div ref={host} class="term-host" />
        </div>
        <Show when={cropped()}>
          <Show when={thumbs().x}>{(x) => <i class="thumb across" style={x()} />}</Show>
          <Show when={thumbs().y}>{(y) => <i class="thumb down" style={y()} />}</Show>
        </Show>
        <selection.Handles />
      </div>
      <textarea
        ref={sink}
        class="sink"
        autocapitalize="off"
        autocomplete="off"
        autocorrect="off"
        spellcheck={false}
        enterkeyhint="enter"
        cols={SINK_COLS}
        rows={SINK_ROWS}
        onFocus={() => {
          resetSink();
          drawFocused(true);
        }}
        onBlur={() => drawFocused(false)}
        onInput={sinkInput}
        onCompositionStart={() => (composing = true)}
        onCompositionEnd={() => {
          composing = false;
          resetSink();
        }}
        onKeyDown={(event) => {
          if (event.key !== "Enter") return;
          event.preventDefault();
          write("\r");
        }}
      />
      <div class="foot" ref={animateList(":scope > *")}>
        <Show when={!props.ended}>
          <Switch>
            <Match when={props.reach.is === "unreachable" && props.reach}>
              {(gone) => (
                <div class="lockbar">
                  <Slot>
                    <Icon name="warn" />
                  </Slot>
                  <span class="name">
                    {serverLabel(props.at.server)} unreachable since {clock(gone().since, "h23")}
                  </span>
                  <Id>retrying</Id>
                </div>
              )}
            </Match>
            <Match when={props.reach.is === "reconnecting" && props.reach}>
              {(kept) => (
                <div class="lockbar">
                  <Slot>
                    <ServerDot server={undefined} reconnecting />
                  </Slot>
                  <span class="name">reconnecting</span>
                  <Id>lock kept {Math.ceil(kept().left / 1000)} s</Id>
                </div>
              )}
            </Match>
            <Match when={props.resumable}>
              <div class="lockbar">
                <span class="name">The agent has ended</span>
                <button class="btn" onClick={() => act(backend.taskFocus(props.at))}>
                  Resume
                </button>
              </div>
            </Match>
            <Match when={mine()}>
              <div class="keys" onPointerDown={(event) => event.preventDefault()}>
                <button class="key" onClick={() => send(ESC)}>
                  esc
                </button>
                <button class="key" onClick={() => send("\t")}>
                  tab
                </button>
                <button class="key" classList={{ on: ctrl() }} onClick={latch}>
                  ctrl
                </button>
                <For each={ARROWS}>
                  {([label, code]) => (
                    <button class="key" onClick={() => send(arrow(code))}>
                      {label}
                    </button>
                  )}
                </For>
                <button class="key" aria-label="Keyboard" onClick={toggleKeyboard}>
                  <Icon name="kbd" />
                </button>
              </div>
            </Match>
            <Match when={props.panel.running && props.panel.lock_holder !== null}>
              <div class="lockbar">
                <span class="name">read-only · held by {props.panel.lock_holder}</span>
                <button class="btn" onClick={session.take}>
                  Take over
                </button>
              </div>
            </Match>
          </Switch>
        </Show>
      </div>
    </div>
  );
}

/** The selected terminal's holder and size, after a coral microphone while it hears this phone, or a red "denied". */
function LockLabel(props: { panel: Panel }) {
  const mine = () => props.panel.lock_holder === me();
  const hearing = () => mine() && props.panel.listening;
  return (
    <span class="lock">
      <Show when={props.panel.running}>
        <Show when={hearing() && micDenied()}>
          <span class="id denied" role="button" onClick={() => phone.settings().catch(notify)}>
            <Icon name="micoff" />
            denied
          </span>
          ·{" "}
        </Show>
        <Show when={hearing() && !micDenied()}>
          <span class="id live">
            <Icon name="mic" />
          </span>
        </Show>
        <Show when={props.panel.lock_holder !== null}>
          <span class="name">{props.panel.lock_holder}</span>· {props.panel.cols}×{props.panel.rows}
        </Show>
      </Show>
    </span>
  );
}

/** What the agent panel says when its harness is missing on the server, with the install command to copy. */
function Missing(props: { open: TaskView }) {
  const install = () => {
    const agent = props.open.task.agent;
    return agent.kind === "harness_not_installed" ? agent.install : null;
  };
  const server = () => serverLabel(props.open.at.server);
  return (
    <div class="missing fill">
      <Gap n={4} />
      <Show
        when={install()}
        fallback={
          <Para>
            {server()} has no harness <b>{props.open.task.harness}</b>.
          </Para>
        }
      >
        {(command) => (
          <>
            <Para>
              <b>{props.open.task.harness}</b> is not installed on {server()}.
            </Para>
            <Gap n={2} />
            <div
              class="cmd wrap"
              role="button"
              onClick={() =>
                navigator.clipboard.writeText(command()).then(() => notify("Copied the install command"), report)
              }
            >
              <span class="name">{command()}</span>
              <Slot>
                <Icon name="copy" />
              </Slot>
            </div>
            <Gap n={2} />
            <Para>The agent starts the next time this task opens.</Para>
          </>
        )}
      </Show>
    </div>
  );
}

function TaskScreen(props: { open: TaskView }) {
  const now = ticking();
  const [chosen, setChosen] = createSignal<PtyId | null>(null);
  const panels = () => props.open.task.panels;
  const agent = () => panels().find((panel) => panel.name === AGENT_PANEL);
  const shells = () => panels().filter((panel) => panel.name !== AGENT_PANEL);
  const selected = () => shells().find((panel) => panel.id === chosen()) ?? agent();
  const choose = (panel: PtyId | null) => {
    if (panel !== chosen()) feel("select");
    setChosen(panel);
  };

  const agentOn = () => selected() === agent();
  const missing = () => ["harness_unknown", "harness_not_installed"].includes(props.open.task.agent.kind);
  const reach = () => reachOf(props.open.at, selected(), now());
  const live = () => reach().is === "reachable";
  const server = () => serverView(props.open.at.server);
  const ended = () => !props.open.live;
  return (
    <>
      <div class="bar">
        <Circle icon="back" label="Tasks" tap={back} />
        <div class="ttl" style={{ "--ws": sectionColor(props.open.section) }}>
          <span class="crumb">
            <i />
            <For each={props.open.crumbs.slice(0, -1)}>
              {(crumb, nth) => (
                <>
                  <Show when={nth() > 0}>
                    <span class="sep">/</span>
                  </Show>
                  {crumb}
                </>
              )}
            </For>
          </span>
          <b class="name">{props.open.task.title}</b>
        </div>
        <Circle icon="more" label="Task menu" tap={() => openSheet({ is: "task", key: props.open.key, open: true })} />
      </div>
      <div class="chips">
        <span
          class="chip"
          classList={{ state: props.open.live }}
          role="button"
          onClick={() => {
            if (!props.open.live) return openSheet({ is: "reopen", key: props.open.key });
            feel("select");
            act(backend.taskMove(props.open.at, props.open.next));
          }}
        >
          {props.open.task.state}
        </span>
        <span class="chip">
          <ServerDot server={server()} reconnecting={reach().is === "reconnecting"} />
          {serverLabel(props.open.at.server)}
          <Id>
            {
              { reachable: `${server()?.rtt_ms ?? "–"} ms`, reconnecting: "reconnecting", unreachable: "unreachable" }[
                reach().is
              ]
            }
          </Id>
        </span>
        <Show when={props.open.task.branch}>
          {(branch) => (
            <span class="chip">
              <Icon name="branch" />
              <span class="mono">{branch()}</span>
            </span>
          )}
        </Show>
        <span class="chip">{props.open.task.harness}</span>
      </div>
      <div class="tabs">
        <div
          class="tabrow"
          ref={(row) => {
            slidingThumb(row);
            animateList(".tab")(row);
          }}
        >
          <span class="tab" data-key="agent" classList={{ on: agentOn() }} role="button" onClick={() => choose(null)}>
            <Glyph status={props.open.glyph} />
            {props.open.task.harness}
          </span>
          <For each={shells()}>
            {(panel) => (
              <span
                class="tab"
                data-key={panel.id}
                classList={{ on: selected()?.id === panel.id }}
                role="button"
                onClick={() => choose(panel.id)}
              >
                {panel.name}
                <span
                  class="x"
                  role="button"
                  aria-label={`Close ${panel.name}`}
                  onClick={(event) => {
                    event.stopPropagation();
                    if (live()) act(backend.termClose(props.open.at, panel.id));
                  }}
                >
                  <Icon name="x" />
                </span>
              </span>
            )}
          </For>
          <Show when={!ended()}>
            <span
              class="tab plus"
              data-key="plus"
              role="button"
              aria-label="New shell"
              onClick={() => live() && act(backend.termOpen(props.open.at, ".").then(setChosen))}
            >
              <Icon name="plus" />
            </span>
          </Show>
        </div>
        <Show when={reach().is !== "unreachable" && selected()}>{(panel) => <LockLabel panel={panel()} />}</Show>
      </div>
      <Show when={agentOn() && agent() === undefined}>
        <Show when={missing()} fallback={<div class="fill" />}>
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
                reach={reach()}
                ended={ended()}
                resumable={!ended() && panel().name === AGENT_PANEL && !panel().running}
              />
            )}
          </Show>
        )}
      </For>
      <Show when={ended()}>
        <div class="lockbar">
          <Slot>
            <Glyph status={props.open.glyph} />
          </Slot>
          <span class="name">
            {props.open.task.state}
            <Show when={props.open.task.ended_at}>{(at) => ` ${ago(at(), now())}`}</Show>
          </span>
          <button class="btn quiet" onClick={() => openSheet({ is: "reopen", key: props.open.key })}>
            Reopen…
          </button>
        </div>
      </Show>
    </>
  );
}

/** The open task, while the model still lists it. */
export function Task(props: { taskKey: string }) {
  return <Show when={model.tasks[props.taskKey]}>{(view) => <TaskScreen open={view()} />}</Show>;
}
