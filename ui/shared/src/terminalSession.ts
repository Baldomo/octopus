import { WebglAddon } from "@xterm/addon-webgl";
import { type ITheme, Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { createEffect, createMemo, createSignal, on, onCleanup } from "solid-js";
import { backend, Channel } from "./backend";
import { type Panel, type TaskRef, TERM_HISTORY_LINES, type TermScreen } from "./generated";
import { lockClaim } from "./lockClaim";
import { sgrMouse } from "./mouseReports";
import { act, holds, report, serverView } from "./state";
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

/** A dropped stream retries after this, by which time a server that went away reads as unreachable. */
const REATTACH_MS = 500;
/** A refusal waits this long for the lock's move that caused it to reach the model before it counts as a problem. */
const LOCK_SETTLES_MS = 500;
/**
 * A pane that holds its size this long has settled. A drag or a rotation resizes the PTY once it settles, since every
 * resize the server takes is published.
 */
const PANE_SETTLES_MS = 150;

/** Whether the app shows anything: a hidden window, or a phone app in the background, shows no terminal. */
export const [windowShown, setWindowShown] = createSignal(true);
const [windowFocuses, setWindowFocuses] = createSignal(0);
export const windowFocused = () => setWindowFocuses((count) => count + 1);

export const readOnlyLabel = (panel: Panel) =>
  panel.lock_holder === null ? "read-only · lock free" : `read-only · held by ${panel.lock_holder.name}`;

type Cells = { cols: number; rows: number };
export type Size = { width: number; height: number };
type Core = { _charSizeService: { measure: () => void }; _renderService: { dimensions: { css: { cell: Size } } } };

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
  sgrMouse: () => boolean;
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
 * it, and its server is reachable; the server reads that stream as this client showing the terminal. A look claims a
 * free lock at the pane's size: the terminal starting to show while its server is reachable, or the window coming into
 * focus while it shows. A stream reopening for the client that held the lock when the last one ended claims too. The holder sizes the PTY to its pane.
 * The server sends the whole screen at the PTY's size on attaching and on every resize, and every client redraws it at
 * that size.
 */
export function terminalSession(props: { at: TaskRef; panel: () => Panel; shown: () => boolean }): TerminalSession {
  const mine = createMemo(() => holds(props.at.server, props.panel().lock_holder));
  const shown = createMemo(props.shown);
  const xterm = new Terminal({
    fontFamily: token("--mono-terminal"),
    fontSize: px("--fs-mono"),
    cursorBlink: true,
    scrollback: TERM_HISTORY_LINES,
    overviewRuler: { width: px("--scrollbar-w") },
  });
  const sgr = sgrMouse(xterm.parser);
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
  const claim = lockClaim();
  let generation = 0;
  /** Counts streams that ended under a running PTY, each of which calls for a fresh attachment. */
  const [drops, setDrops] = createSignal(0);

  /** A screen resets the terminal to its size before its bytes draw it; an empty message ends the stream. */
  const attach = async () => {
    const current = ++generation;
    const output = new Channel<TermScreen | ArrayBuffer>();
    let ended = false;
    output.onmessage = (piece) => {
      if (current !== generation) return;
      if (!(piece instanceof ArrayBuffer)) {
        xterm.write(new Uint8Array(), () => {
          xterm.reset();
          sgr.reset();
          xterm.resize(piece.cols, piece.rows);
        });
        return;
      }
      if (piece.byteLength === 0) {
        ended = true;
        attachment = null;
        claim.ended(mine());
        setAttached(false);
        if (props.panel().running) setTimeout(() => setDrops((count) => count + 1), REATTACH_MS);
        return;
      }
      xterm.write(new Uint8Array(piece));
    };
    const opened = await backend.termAttach(props.at, props.panel().id, output);
    if (current === generation && !ended) {
      attachment = opened;
      setAttached(true);
      if (claim.opened()) claimUnlessTaken();
    } else if (current !== generation) act(backend.termDetach(opened));
  };
  const detach = () => {
    generation++;
    if (attachment !== null) act(backend.termDetach(attachment));
    attachment = null;
    claim.ended(mine());
    setAttached(false);
  };
  /**
   * A resize or claim another client's lock outran is moot rather than a problem: the terminal shows that client as
   * its holder once the lock's move reaches the model.
   */
  const unlessOutrun = (sent: Promise<void>, stillMine: () => boolean) =>
    sent.catch((error) => setTimeout(() => stillMine() && report(error), LOCK_SETTLES_MS));
  let fitting: ReturnType<typeof setTimeout> | undefined;
  const fit = () => {
    clearTimeout(fitting);
    fitting = setTimeout(() => {
      const size = pane();
      const { id, cols, rows } = props.panel();
      if (!mine() || !size || (size.cols === cols && size.rows === rows)) return;
      unlessOutrun(backend.termResize(props.at, id, size.cols, size.rows), mine);
    }, PANE_SETTLES_MS);
  };
  const atPaneSize = (send: (size: Cells) => Promise<void>, what: string, stillMine: () => boolean) => {
    const size = pane();
    if (size) unlessOutrun(send(size), stillMine);
    else report(`the terminal's pane has no size to ${what} the terminal at yet`);
  };
  const takenByOther = () => {
    const holder = props.panel().lock_holder;
    return holder !== null && !holds(props.at.server, holder);
  };
  const claimUnlessTaken = () => {
    const { id, running } = props.panel();
    if (takenByOther() || !running) return;
    atPaneSize(
      (size) => backend.termClaim(props.at, id, size.cols, size.rows),
      "claim",
      () => !takenByOther(),
    );
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
    on([() => shown() && windowShown(), windowFocuses], ([looking]) => {
      if (!looking) return;
      if (attached()) claimUnlessTaken();
      else if (streaming()) claim.looked();
    }),
  );
  createEffect(
    on([mine, shown], ([holding, visible]) => {
      if (visible && holding) fit();
    }),
  );
  onCleanup(() => {
    clearTimeout(fitting);
    detach();
    xterm.dispose();
  });

  return {
    xterm,
    mine,
    shown,
    sgrMouse: sgr.on,
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
