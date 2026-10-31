import { WebglAddon } from "@xterm/addon-webgl";
import { type ITheme, Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { createEffect, createMemo, createSignal, on, onCleanup } from "solid-js";
import { backend, Channel } from "./backend";
import { type Panel, type TaskRef, TERM_SIZE_OSC } from "./generated";
import { act, me, report, serverView } from "./state";
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

const SCROLLBACK_LINES = 10_000;
/** A dropped stream retries after this, by which time a server that went away reads as unreachable. */
const REATTACH_MS = 500;
/** A refusal waits this long for the lock's move that caused it to reach the model before it counts as a problem. */
const LOCK_SETTLES_MS = 500;

/** Whether the app shows anything: a hidden window, or a phone app in the background, shows no terminal. */
export const [windowShown, setWindowShown] = createSignal(true);

type Cells = { cols: number; rows: number };
export type Size = { width: number; height: number };
type Core = { _charSizeService: { measure: () => void }; _renderService: { dimensions: { css: { cell: Size } } } };
/** A terminal's size record, written by its server as `<cols>;<rows>`. */
const SIZE_RECORD = /^(\d+);(\d+)$/;
/** A program can print a size record too; past this many cells a side, one would freeze the client allocating it. */
const MOST_CELLS = 1000;

/**
 * The cell xterm draws, which only its private services know. It measures afresh, since xterm measures a terminal that
 * opened hidden only once it sees it on screen.
 */
export function cellSize(xterm: Terminal): Size {
  const core = (xterm as unknown as { _core: Core })._core;
  core._charSizeService.measure();
  return core._renderService.dimensions.css.cell;
}

export type TerminalSession = {
  xterm: Terminal;
  mine: () => boolean;
  shown: () => boolean;
  /** Writes `keys` to the terminal while this client streams it. */
  input: (keys: string) => void;
  /** Moves the lock here at the pane's size. */
  take: () => void;
  /** Sizes the terminal to its pane while this client holds the lock. */
  fit: () => void;
  /** Opens the terminal in `pane`, the element whose size is the pane's, drawing through WebGL where it can. */
  open: (pane: HTMLElement) => void;
};

/**
 * A terminal keeps its screen while it stays mounted, and holds a data stream only while it is `shown`, the app shows
 * it, and its server is reachable; the server reads that stream as this client showing the terminal. A shown terminal
 * with a free lock is claimed at the pane's size. The holder sizes the PTY to its pane. The output carries the PTY's
 * size wherever it changes, and every client draws the terminal at the size it reads there, so each byte lands on the
 * screen it was written for.
 */
export function terminalSession(props: { at: TaskRef; panel: () => Panel; shown: () => boolean }): TerminalSession {
  const mine = createMemo(() => props.panel().lock_holder === me());
  const shown = createMemo(props.shown);
  const xterm = new Terminal({
    fontFamily: token("--mono-terminal"),
    fontSize: px("--fs-mono"),
    cursorBlink: true,
    scrollback: SCROLLBACK_LINES,
    overviewRuler: { width: px("--scrollbar-w") },
  });
  xterm.parser.registerOscHandler(TERM_SIZE_OSC, (record) => {
    const size = SIZE_RECORD.exec(record);
    const [cols, rows] = [Number(size?.[1]), Number(size?.[2])];
    if (cols >= 1 && cols <= MOST_CELLS && rows >= 1 && rows <= MOST_CELLS) xterm.resize(cols, rows);
    return true;
  });
  /** The cells that fit the element the terminal opened in, beside the overview ruler along its right edge. */
  const pane = (): Cells | undefined => {
    const host = xterm.element?.parentElement;
    if (!host?.clientWidth || !host.clientHeight) return undefined;
    const cell = cellSize(xterm);
    if (!cell.width || !cell.height) return undefined;
    return {
      cols: Math.floor((host.clientWidth - (xterm.options.overviewRuler?.width ?? 0)) / cell.width),
      rows: Math.floor(host.clientHeight / cell.height),
    };
  };
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
        if (props.panel().running) setTimeout(() => setDrops((count) => count + 1), REATTACH_MS);
        return;
      }
      if (replaying) {
        xterm.reset();
        replaying = false;
      }
      xterm.write(new Uint8Array(bytes));
    };
    const opened = await backend.termAttach(props.at, props.panel().id, output);
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
  /**
   * A resize or claim another client's lock outran is moot rather than a problem: the terminal shows that client as
   * its holder once the lock's move reaches the model.
   */
  const unlessOutrun = (sent: Promise<void>, stillMine: () => boolean) =>
    sent.catch((error) => setTimeout(() => stillMine() && report(error), LOCK_SETTLES_MS));
  /** A drag or a rotation resizes once per frame, since every resize the server takes is published. */
  let fitting = 0;
  const fit = () => {
    cancelAnimationFrame(fitting);
    fitting = requestAnimationFrame(() => {
      const size = pane();
      const { id, cols, rows } = props.panel();
      if (!mine() || !size || (size.cols === cols && size.rows === rows)) return;
      unlessOutrun(backend.termResize(props.at, id, size.cols, size.rows), mine);
    });
  };
  const atPaneSize = (send: (size: Cells) => Promise<void>, what: string, stillMine: () => boolean) => {
    const size = pane();
    if (size) unlessOutrun(send(size), stillMine);
    else report(`the terminal's pane has no size to ${what} the terminal at yet`);
  };

  const streaming = createMemo(() => shown() && windowShown() && serverView(props.at.server)?.reachable === true);
  createEffect(
    on([streaming, drops], ([stream]) => {
      if (!stream) return;
      attach().catch(report);
      onCleanup(detach);
    }),
  );
  createEffect(() => {
    xterm.options.theme = terminalTheme(scheme());
  });
  createEffect(
    on([attached, () => props.panel().lock_holder === null && props.panel().running], ([showing, free]) => {
      if (showing && free) {
        atPaneSize(
          (size) => backend.termClaim(props.at, props.panel().id, size.cols, size.rows),
          "claim",
          () => props.panel().lock_holder === null,
        );
      }
    }),
  );
  createEffect(
    on([mine, shown], ([holding, visible]) => {
      if (visible && holding) fit();
    }),
  );
  onCleanup(() => {
    cancelAnimationFrame(fitting);
    detach();
    xterm.dispose();
  });

  return {
    xterm,
    mine,
    shown,
    input: (keys) => {
      if (attachment !== null) act(backend.termInput(attachment, keys));
    },
    take: () =>
      atPaneSize(
        (size) => backend.termTakeLock(props.at, props.panel().id, size.cols, size.rows),
        "take",
        () => true,
      ),
    fit,
    open: (pane) => {
      xterm.open(pane);
      try {
        const webgl = new WebglAddon();
        webgl.onContextLoss(() => webgl.dispose());
        xterm.loadAddon(webgl);
      } catch (error) {
        report(`WebGL is unavailable, so terminals draw through the DOM: ${error}`);
      }
    },
  };
}
