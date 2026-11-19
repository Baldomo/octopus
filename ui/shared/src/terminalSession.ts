import { WebglAddon } from "@xterm/addon-webgl";
import { type ITheme, Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { createEffect, createMemo, createSignal, on, onCleanup } from "solid-js";
import { backend, Channel } from "./backend";
import { type PanelView, type TaskRef, TERM_HISTORY_LINES, type TermScreen } from "./generated";
import { lockClaim } from "./lockClaim";
import { sgrMouse } from "./mouseReports";
import { act, report, serverView } from "./state";
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

/** By this time a server that went away reads as unreachable. */
const REATTACH_MS = 500;
/** A refusal waits this long for the lock's move that caused it to reach the model before it counts as a problem. */
const LOCK_SETTLES_MS = 500;
/** Every resize the server takes is published, so a drag or a rotation resizes the PTY only once it settles. */
const PANE_SETTLES_MS = 150;

export const [windowShown, setWindowShown] = createSignal(true);
const [windowFocuses, setWindowFocuses] = createSignal(0);
export const windowFocused = () => setWindowFocuses((count) => count + 1);

type Cells = { cols: number; rows: number };
export type Size = { width: number; height: number };
type Core = {
  _charSizeService: { measure: () => void };
  _renderService: {
    dimensions: { css: { cell: Size } };
    _handleIntersectionChange: (entry: { intersectionRatio: number }) => void;
  };
};
const core = (xterm: Terminal) => (xterm as unknown as { _core: Core })._core;

/** Measures afresh, since xterm measures a terminal that opened hidden only once it sees it on screen. */
export function cellSize(xterm: Terminal): Size {
  core(xterm)._charSizeService.measure();
  return core(xterm)._renderService.dimensions.css.cell;
}

/** Draws a terminal just shown in the coming frame, which xterm would paint blank until its visibility observer reports. */
const paintNextFrame = (xterm: Terminal) =>
  core(xterm)._renderService._handleIntersectionChange({ intersectionRatio: 1 });

export function terminalSession(props: { at: TaskRef; panel: () => PanelView; shown: () => boolean }) {
  const mine = createMemo(() => props.panel().held_here);
  const shown = createMemo(props.shown);
  const xterm = new Terminal({
    fontFamily: token("--mono-terminal"),
    fontSize: px("--fs-mono"),
    cursorBlink: true,
    scrollback: TERM_HISTORY_LINES,
    overviewRuler: { width: px("--scrollbar-w") },
  });
  const sgr = sgrMouse(xterm.parser);
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
  /** Holding the lock or claiming it, so a look draws the terminal as held here from the first frame. */
  const claimed = () => mine() || claim.pending();
  let generation = 0;
  const [drops, setDrops] = createSignal(0);

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
      if (claim.opened()) claimIfFree();
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
  const sentAtPaneSize = (send: (size: Cells) => Promise<void>, what: string, stillMine: () => boolean) => {
    const size = pane();
    if (size) unlessOutrun(send(size), stillMine);
    else report(`the terminal's pane has no size to ${what} the terminal at yet`);
    return size !== undefined;
  };
  const takenByOther = () => {
    const { lock_holder, held_here } = props.panel();
    return lock_holder !== null && !held_here;
  };
  const claimable = () => !takenByOther() && props.panel().running;
  const claimIfFree = () => {
    const sent =
      claimable() &&
      sentAtPaneSize(
        (size) =>
          backend.termClaim(props.at, props.panel().id, size.cols, size.rows).catch((error) => {
            claim.settled();
            throw error;
          }),
        "claim",
        () => !takenByOther(),
      );
    if (!sent) claim.settled();
  };
  createEffect(
    on(
      [() => props.panel().lock_holder?.name, () => props.panel().held_here, () => props.panel().running],
      claim.settled,
      { defer: true },
    ),
  );

  const looking = createMemo(() => shown() && windowShown());
  const streaming = createMemo(() => looking() && serverView(props.at.server)?.reachable === true);
  createEffect(
    on([streaming, drops], ([stream]) => {
      if (!stream) return;
      attach().catch((error) => {
        claim.ended(false);
        report(error);
      });
      onCleanup(detach);
    }),
  );
  createEffect(() => {
    xterm.options.theme = terminalTheme(scheme());
  });
  createEffect(
    on([looking, windowFocuses], ([visible]) => {
      if (!visible || !streaming() || !claimable()) return;
      claim.looked();
      if (attached() && claim.opened()) claimIfFree();
    }),
  );
  createEffect(
    on([mine, shown], ([holding, visible]) => {
      if (visible && holding) fit();
    }),
  );

  /**
   * Off screen a terminal keeps only its screen and draws through the DOM: the next attach replays its history from
   * the server, and its WebGL context goes back to the GPU.
   */
  let opened = false;
  let webgl: WebglAddon | null = null;
  let webglWorks = true;
  const loadWebgl = () => {
    try {
      const addon = new WebglAddon();
      addon.onContextLoss(() => {
        addon.dispose();
        if (webgl === addon) webgl = null;
      });
      xterm.loadAddon(addon);
      webgl = addon;
    } catch (error) {
      webglWorks = false;
      report(`WebGL is unavailable, so terminals draw through the DOM: ${error}`);
    }
  };
  const draw = () => {
    if (!opened) return;
    if (looking()) {
      xterm.options.scrollback = TERM_HISTORY_LINES;
      if (webgl === null && webglWorks) loadWebgl();
      paintNextFrame(xterm);
    } else {
      webgl?.dispose();
      webgl = null;
      xterm.options.scrollback = 0;
    }
  };
  createEffect(on(looking, draw));

  onCleanup(() => {
    clearTimeout(fitting);
    detach();
    xterm.dispose();
  });

  return {
    xterm,
    mine,
    claimed,
    shown,
    sgrMouse: sgr.on,
    input: (keys: string) => {
      if (attachment !== null) act(backend.termInput(attachment, keys));
    },
    take: () => {
      const sent = sentAtPaneSize(
        (size) =>
          backend.termTakeLock(props.at, props.panel().id, size.cols, size.rows).catch((error) => {
            claim.settled();
            throw error;
          }),
        "take",
        () => true,
      );
      if (sent) claim.took();
      return sent;
    },
    fit,
    open: (pane: HTMLElement) => {
      xterm.open(pane);
      opened = true;
      draw();
    },
  };
}
