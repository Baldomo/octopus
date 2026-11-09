import { createEffect, createSignal, For, Match, on, onCleanup, onMount, Show, Switch } from "solid-js";
import { Portal } from "solid-js/web";
import { backend } from "@octopus/shared/backend.ts";
import { AGENT_PANEL, type PanelView, type PtyId, type TaskRef, type TaskView } from "@octopus/shared/generated.ts";
import { ago, clock, ticking } from "@octopus/shared/time.ts";
import { act, model, scale, serverLabel, serverView, ui } from "@octopus/shared/state.ts";
import { clickReport, type ScreenCell, wheelReport } from "@octopus/shared/mouseReports.ts";
import { cellSize, type Size, terminalSession } from "@octopus/shared/terminalSession.ts";
import { colors, scheme } from "@octopus/shared/tokens.ts";
import { feel } from "./feel";
import { animateList } from "@octopus/shared/flip.ts";
import { EachPanel, taskPanels } from "@octopus/shared/panels.ts";
import { Icon } from "./Icon";
import { onPainted } from "./motion";
import { micDenied, openSheet } from "./nav";
import { Circle, CrumbBar, Gap, Glyph, Id, Para, sectionColor, ServerDot, Slot, Trail } from "./parts";
import { phone } from "./phone";
import { repeats, type Point } from "./press";
import { type Reach, reachOf } from "./reach";
import { type Scroll, scroller } from "./scroll";
import { TERMINAL_FONT } from "./symbols";
import { type Cell, charAt, termSelection, wrappedLine } from "./termSelect";
import { slidingThumb } from "./thumb";
import { copy, notify } from "./Toast";
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
const FONT_PX = 13;
const ROW_PX = 20;
/** How near a line boundary, in lines, the terminal counts as on it, so float error never rests it a line off. */
const ON_LINE = 1e-6;
/** A drag on a program's own scroll starts this far from either end of its endless axis, further than a throw goes. */
const ENDLESS_PX = 1e6;
/** A program's own scroll that sent a row this recently is still moving, so a touch on it stops it rather than taps. */
const TURNING_MS = 100;

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

/**
 * One terminal panel. The holder types into it through the keyboard and the key bar and sizes it to the pane; everyone
 * else sees it at the holder's size, cropped to the pane at its last line, panned by dragging.
 */
function PhoneTerm(props: {
  at: TaskRef;
  panel: PanelView;
  visible: boolean;
  reach: Reach;
  ended: boolean;
  resumable: boolean;
  tail: HTMLElement | undefined;
  typed: boolean;
  typing: (typing: boolean) => void;
}) {
  let host!: HTMLDivElement;
  let pane!: HTMLDivElement;
  let sink!: HTMLTextAreaElement;
  const [opened, setOpened] = createSignal(false);
  const session = terminalSession({ at: props.at, panel: () => props.panel, shown: () => props.visible && opened() });
  const { xterm, mine, claimed } = session;
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
  const cropped = () => !claimed() && (screen().width > view().width || screen().height > view().height);
  const most = () => ({
    x: Math.max(screen().width - view().width, 0),
    y: Math.max(screen().height - view().height, 0),
  });

  const write = (keys: string) => {
    toLastLine();
    session.input(keys);
  };
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
  createEffect(() => {
    if (!props.visible || !mine() || !live() || props.ended || props.resumable) sink.blur();
  });
  onCleanup(() => {
    if (document.activeElement === sink) props.typing(false);
  });
  createEffect(
    on(
      () => {
        const { running, lock_holder, held_here } = props.panel;
        return props.visible && running && lock_holder !== null ? held_here : null;
      },
      (mine, was) => {
        if (mine === null || was === null || was === undefined || mine === was) return;
        feel(mine ? "success" : "warning");
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
  const tracksMouse = () => xterm.modes.mouseTrackingMode !== "none";
  const onScreen = (cell: Cell): ScreenCell => ({ col: cell.col, row: cell.row - xterm.buffer.active.viewportY });
  /**
   * A tap places the cursor: a program tracking the mouse hears a left click at the tapped cell, and on the normal
   * screen a tap on the cursor's line steps the cursor there by ← or →, stopping at the end of the line's text.
   */
  const placeCursor = (point: Point) => {
    const tapped = selection.cellAt(point, false);
    const buffer = xterm.buffer.active;
    if (tapped === null) return;
    if (tracksMouse()) return write(clickReport(onScreen(tapped), session.sgrMouse()));
    if (buffer.type !== "normal") return;
    const cursorCell = { col: buffer.cursorX, row: buffer.baseY + buffer.cursorY };
    const line = wrappedLine(xterm, cursorCell.row);
    if (!line.some((char) => char.cell.row === tapped.row)) return;
    const cursor = charAt(line, cursorCell);
    const textEnd = line.findLastIndex((char) => char.text !== " ") + 1;
    const steps = Math.min(charAt(line, tapped), Math.max(cursor, textEnd)) - cursor;
    if (steps !== 0) write(arrows(steps, "D", "C"));
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
  /**
   * A program on the alternate screen or tracking the mouse scrolls itself: it hears a wheel turn at the touch's cell
   * while it tracks the mouse, else ↑ or ↓, for each row a drag or its throw travels.
   */
  const scrollsItself = () => mine() && (tracksMouse() || xterm.buffer.active.type === "alternate");
  let turned = ENDLESS_PX;
  let lastTurnAt = -Infinity;
  let turnedAt: ScreenCell = { col: 0, row: 0 };
  const turns = scroller({
    at: () => turned,
    end: () => 2 * ENDLESS_PX,
    rest: (at) => at,
    apply: (at) => {
      const rows = Math.floor(at / cell()) - Math.floor(turned / cell());
      turned = at;
      if (rows === 0 || !scrollsItself()) return;
      lastTurnAt = performance.now();
      const way = rows < 0 ? -1 : 1;
      session.input(
        tracksMouse() ? wheelReport(way, turnedAt, session.sgrMouse()).repeat(Math.abs(rows)) : arrows(rows, "A", "B"),
      );
    },
  });
  const turn = (from: Point, at: Point) => {
    const landed = selection.cellAt(from, true);
    if (landed === null) return null;
    turnedAt = onScreen(landed);
    turned = ENDLESS_PX;
    const y = turns.grab();
    scrolling = true;
    return {
      move: (to: Point) => y.drag(at.y - to.y),
      end: (velocity: Point | null) => y.release(-(velocity?.y ?? 0)),
    };
  };
  const drag: Gesture = ({ axis, from, at }) => {
    if (!measured()) return null;
    if (scrollsItself()) return axis === "y" ? turn(from, at) : null;
    if (axis === "x" && !cropped()) return null;
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
  /** A touch that stops a program's own scroll only stops it, so lifting it clicks nothing. */
  let stoppedTurning = false;
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
  createEffect(() => {
    const color = colors(scheme());
    xterm.options.theme = {
      ...xterm.options.theme,
      background: color("--bg"),
      foreground: color(claimed() ? "--term-text" : "--dim"),
      cursor: color("--text"),
      cursorAccent: color("--bg"),
      selectionBackground: color("--select"),
    };
  });
  createEffect(() => {
    const { x, y } = pan();
    if (opened()) xterm.element!.style.transform = `translate(${-x}px, ${-y}px)`;
  });
  createEffect(() => setPan(cropped() ? { x: 0, y: most().y } : { x: 0, y: 0 }));

  const thumbs = () => {
    const whole = screen();
    const seen = view();
    const across = Math.min(seen.width / whole.width, 1);
    const down = Math.min(seen.height / whole.height, 1);
    return {
      x: across < 1 && {
        left: `calc(var(--g1) + ${(pan().x / whole.width) * seen.width}px / var(--zoom, 1))`,
        width: `calc(${across * seen.width}px / var(--zoom, 1))`,
      },
      y: down < 1 && {
        top: `calc(var(--g1) + ${(pan().y / whole.height) * seen.height}px / var(--zoom, 1))`,
        height: `calc(${down * seen.height}px / var(--zoom, 1))`,
      },
    };
  };

  return (
    <div class="tterm fill" style={{ display: props.visible ? "" : "none" }}>
      <div
        ref={pane}
        class="term"
        classList={{ held: !claimed(), stale: !live() }}
        onTouchStart={(event) => {
          const touch = event.touches[0];
          from = touch === undefined ? null : { x: touch.clientX, y: touch.clientY };
          scrolling = false;
          stoppedTurning = performance.now() - lastTurnAt < TURNING_MS;
          down.stop();
          across.stop();
          turns.stop();
        }}
        onTouchCancel={settle}
        onTouchEnd={(event) => {
          settle();
          if (event.defaultPrevented || stoppedTurning || selection.onControls(event)) return;
          const touch = event.changedTouches[0];
          if (!from || !touch || Math.hypot(touch.clientX - from.x, touch.clientY - from.y) >= COMMIT_PX) return;
          const point = { x: touch.clientX, y: touch.clientY };
          const tapped = selection.tap(point);
          if (!tapped && (!mine() || !live())) return;
          // The click a tap synthesizes would land on the pane and take the focus back from the keyboard.
          event.preventDefault();
          if (tapped) return;
          placeCursor(point);
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
          props.typing(true);
        }}
        onBlur={() => {
          drawFocused(false);
          setCtrl(false);
          props.typing(false);
        }}
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
      <Show when={props.visible && props.tail}>
        {(tail) => (
          <Portal mount={tail()}>
            <Show when={!props.ended && props.reach.is !== "unreachable"}>
              <Switch>
                <Match when={props.reach.is === "reconnecting" && props.reach}>
                  {(kept) => <Id>lock kept {Math.ceil(kept().left / 1000)} s</Id>}
                </Match>
                <Match when={props.resumable}>
                  <button class="btn" onClick={() => act(backend.taskFocus(props.at))}>
                    Resume
                  </button>
                </Match>
                <Match when={mine()}>
                  <Show when={props.typed}>
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
                        <button class="key" ref={repeats(() => write(arrow(code)))}>
                          {label}
                        </button>
                      )}
                    </For>
                  </Show>
                  <button class="key" aria-label="Keyboard" onClick={toggleKeyboard}>
                    <Icon name="kbd" />
                  </button>
                </Match>
                <Match when={claimed()}>{null}</Match>
                <Match when={props.panel.running}>
                  <Id>{props.panel.lock_holder?.name ?? "lock free"}</Id>
                  <button class="btn" onClick={session.take}>
                    Take over
                  </button>
                </Match>
              </Switch>
            </Show>
          </Portal>
        )}
      </Show>
    </div>
  );
}

function Mic(props: { at: TaskRef; panel: PanelView }) {
  const hearing = () => props.panel.running && props.panel.held_here && props.panel.listening;
  return (
    <Show when={hearing()}>
      <Show
        when={micDenied()}
        fallback={
          <span class="mic">
            <Icon name="mic" big />
          </span>
        }
      >
        <button class="mic denied" aria-label="Microphone settings" onClick={() => phone.settings().catch(notify)}>
          <Icon name="micoff" big />
        </button>
      </Show>
    </Show>
  );
}

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
            <div class="cmd wrap" role="button" onClick={() => act(copy(command(), "the install command"))}>
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
  const { panels, agent, shells, selected, agentOn } = taskPanels(() => props.open, chosen);
  const choose = (panel: PtyId | null) => {
    if (panel !== chosen()) feel("select");
    setChosen(panel);
  };

  const missing = () => ["harness_unknown", "harness_not_installed"].includes(props.open.task.agent.kind);
  const reach = () => reachOf(props.open.at, selected(), now());
  const live = () => reach().is === "reachable";
  const server = () => serverView(props.open.at.server);
  const ended = () => !props.open.live;
  const gone = () => {
    const reached = reach();
    return reached.is === "unreachable" && !ended() ? reached : null;
  };
  const [typing, setTyping] = createSignal(false);
  const [tail, setTail] = createSignal<HTMLElement>();
  return (
    <>
      <CrumbBar
        slim
        up="Tasks"
        color={sectionColor(props.open.section)}
        crumb={
          <>
            <span class="name">
              <Trail parts={props.open.crumbs.slice(0, -1)} />
            </span>
            <span class="srv">
              <ServerDot server={server()} reconnecting={reach().is === "reconnecting"} />
              {serverLabel(props.open.at.server)}
            </span>
          </>
        }
        title={props.open.task.title}
        end={
          <>
            <Show when={reach().is !== "unreachable" && selected()}>
              {(panel) => <Mic at={props.open.at} panel={panel()} />}
            </Show>
            <Circle
              icon="more"
              label="Task menu"
              tap={() => openSheet({ is: "task", key: props.open.key, open: { panel: selected()?.id ?? null } })}
            />
          </>
        }
      />
      <Show when={agentOn() && agent() === undefined}>
        <Show when={missing()} fallback={<div class="fill" />}>
          <Missing open={props.open} />
        </Show>
      </Show>
      <EachPanel panels={panels()}>
        {(panel) => (
          <PhoneTerm
            at={props.open.at}
            panel={panel()}
            visible={selected()?.id === panel().id}
            reach={reach()}
            ended={ended()}
            resumable={!ended() && panel().name === AGENT_PANEL && !panel().running}
            tail={tail()}
            typed={typing()}
            typing={setTyping}
          />
        )}
      </EachPanel>
      <div class="foot" ref={animateList(":scope > *")}>
        <Show when={gone()}>
          {(down) => (
            <div class="lockbar">
              <Slot>
                <Icon name="warn" />
              </Slot>
              <span class="name">
                {serverLabel(props.open.at.server)} unreachable since {clock(down().since, "h23")}
              </span>
              <Id>retrying</Id>
            </div>
          )}
        </Show>
        <div class="dock" classList={{ typing: typing() }}>
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
          <span class="tail" ref={setTail} on:pointerdown={(event) => event.preventDefault()}>
            <Show when={ended()}>
              <Show when={props.open.task.ended_at}>{(at) => <Id>{ago(at(), now())}</Id>}</Show>
              <button class="btn quiet" onClick={() => openSheet({ is: "reopen", key: props.open.key })}>
                Reopen…
              </button>
            </Show>
          </span>
        </div>
      </div>
    </>
  );
}

export function Task(props: { taskKey: string }) {
  return <Show when={model.tasks[props.taskKey]}>{(view) => <TaskScreen open={view()} />}</Show>;
}
