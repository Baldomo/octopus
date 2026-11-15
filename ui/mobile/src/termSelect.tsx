import type { Terminal } from "@xterm/xterm";
import { createEffect, createSignal, onCleanup, Show } from "solid-js";
import { scale, ui } from "@octopus/shared/state.ts";
import { urlsIn } from "@octopus/shared/terminalUrls.ts";
import { px } from "@octopus/shared/tokens.ts";
import { feel } from "./feel";
import type { Pasted } from "./phone";
import { LONG_PRESS_MS, type Point } from "./press";
import { type Box, SelectionMenu } from "./selectionMenu";
import { COMMIT_PX, fingerIn, keepTouch } from "./touch";

/** A buffer cell; `row` counts from the first line of the scrollback. */
export type Cell = { col: number; row: number };
/** Both ends inclusive, `from` first. */
type Span = { from: Cell; to: Cell };
type Size = { width: number; height: number };
/** One character of a wrapped line and the cell it starts at; a wide character takes two cells. */
type Char = { text: string; cell: Cell; width: number };

/** xterm's double-click word separators, so a word reads the same on the phone and the desktop. */
const SEPARATORS = " ()[]{}',\"`";
const before = (a: Cell, b: Cell) => a.row < b.row || (a.row === b.row && a.col < b.col);
const ordered = (a: Cell, b: Cell): Span => (before(b, a) ? { from: b, to: a } : { from: a, to: b });

/**
 * Whether `row` continues the line above it: xterm marks it wrapped, or the row above runs text to the right edge, as a
 * shell that erases the row it wrapped onto leaves it, which zsh does.
 */
function continues(xterm: Terminal, row: number): boolean {
  const buffer = xterm.buffer.active;
  const edge =
    buffer
      .getLine(row - 1)
      ?.getCell(xterm.cols - 1)
      ?.getChars() ?? "";
  return buffer.getLine(row)?.isWrapped === true || edge.trim() !== "";
}

export function wrappedLine(xterm: Terminal, row: number): Char[] {
  const buffer = xterm.buffer.active;
  let first = row;
  while (first > 0 && continues(xterm, first)) first--;
  let last = row;
  while (last + 1 < buffer.length && continues(xterm, last + 1)) last++;
  const chars: Char[] = [];
  for (let at = first; at <= last; at++) {
    const line = buffer.getLine(at);
    for (let col = 0; col < xterm.cols; col++) {
      const cell = line?.getCell(col);
      const width = cell?.getWidth() ?? 1;
      if (width > 0) chars.push({ text: cell?.getChars() || " ", cell: { col, row: at }, width });
    }
  }
  return chars;
}

export const charAt = (chars: Char[], cell: Cell) => chars.findLastIndex((char) => !before(cell, char.cell));
const lastCell = (char: Char): Cell => ({ col: char.cell.col + char.width - 1, row: char.cell.row });

/** The word covering `cell`, or the one character when it is a separator other than a space; null on a space. */
function wordAt(xterm: Terminal, cell: Cell): Span | null {
  const chars = wrappedLine(xterm, cell.row);
  const inWord = (nth: number) => chars[nth] !== undefined && !SEPARATORS.includes(chars[nth].text);
  let first = charAt(chars, cell);
  let last = first;
  if (inWord(first)) {
    while (inWord(first - 1)) first--;
    while (inWord(last + 1)) last++;
  }
  const [start, end] = [chars[first], chars[last]];
  return start && end && start.text !== " " ? { from: start.cell, to: lastCell(end) } : null;
}

function linkAt(xterm: Terminal, cell: Cell): string | null {
  const chars = wrappedLine(xterm, cell.row);
  const starts: number[] = [];
  let text = "";
  for (const char of chars) {
    starts.push(text.length);
    text += char.text;
  }
  const offset = starts[charAt(chars, cell)];
  if (offset === undefined) return null;
  return urlsIn(text).find(({ url, at }) => offset >= at && offset < at + url.length)?.url ?? null;
}

/**
 * Touch selection over xterm's selection API, which xterm itself drives only by mouse. A finger maps to a cell through
 * the drawn screen's box on the page; handles and the menu sit in the pane's points, xterm's pixels divided by the zoom.
 */
export function termSelection(props: {
  xterm: Terminal;
  /** The drawn screen, wherever the pan has moved it; absent until the terminal first paints. */
  screen: () => HTMLElement | undefined;
  /** The drawn screen's size, and the part of it the pane shows, in xterm's pixels. */
  size: () => Size;
  view: () => Size;
  pan: () => Point;
  /** Whether the terminal's tab is chosen and its Task screen is on top. */
  shown: () => boolean;
  /** Whether the menu offers Paste, and what to do with what it pastes. */
  canPaste: () => boolean;
  paste: (pasted: Pasted) => void;
  /** Opens a URL the finger taps. */
  open: (url: string) => void;
}) {
  const { xterm } = props;
  /** Read again on every frame xterm draws, since scrolling and trimmed scrollback move it without a change event. */
  const [span, setSpan] = createSignal<Span | null>(null, { equals: false });
  /** A finger making or moving the selection keeps the menu out of its way. */
  const [moving, setMoving] = createSignal(false);
  /** Where a hold that selected nothing shows the menu. */
  const [spot, setSpot] = createSignal<Cell | null>(null);

  const readSelection = () => {
    const at = xterm.getSelectionPosition();
    setSpan(
      at === undefined
        ? null
        : { from: { col: at.start.x, row: at.start.y }, to: { col: at.end.x - 1, row: at.end.y } },
    );
  };
  const dismiss = () => {
    xterm.clearSelection();
    setSpot(null);
  };
  /** xterm skips onSelectionChange for a selection equal to the last one it announced, even after a clear. */
  const select = ({ from, to }: Span) => {
    xterm.select(from.col, from.row, (to.row - from.row) * xterm.cols + to.col - from.col + 1);
    readSelection();
  };
  const listeners = [
    xterm.onSelectionChange(readSelection),
    xterm.onRender(readSelection),
    xterm.onResize(() => dismiss()),
  ];
  onCleanup(() => listeners.forEach((listener) => listener.dispose()));
  createEffect(() => {
    if (!props.shown()) dismiss();
  });

  /** The cell under `at`, or null off the drawn screen unless `clamped` to its nearest cell. */
  const cellAt = (at: Point, clamped: boolean): Cell | null => {
    const box = props.screen()?.getBoundingClientRect();
    if (box === undefined) return null;
    const across = Math.floor(((at.x - box.left) / box.width) * xterm.cols);
    const down = Math.floor(((at.y - box.top) / box.height) * xterm.rows);
    const inside = across >= 0 && across < xterm.cols && down >= 0 && down < xterm.rows;
    if (!inside && !clamped) return null;
    return {
      col: Math.min(Math.max(across, 0), xterm.cols - 1),
      row: Math.min(Math.max(down, 0), xterm.rows - 1) + xterm.buffer.active.viewportY,
    };
  };
  const middleOf = (cell: Cell): Point => {
    const box = props.screen()?.getBoundingClientRect() ?? new DOMRect();
    return {
      x: box.left + ((cell.col + 0.5) / xterm.cols) * box.width,
      y: box.top + ((cell.row - xterm.buffer.active.viewportY + 0.5) / xterm.rows) * box.height,
    };
  };
  /** The cell's top left corner in points from the terminal's origin in the pane. */
  const cornerOf = (cell: Cell): Point => {
    const zoom = scale(ui.prefs);
    const { width, height } = props.size();
    return {
      x: ((cell.col * width) / xterm.cols - props.pan().x) / zoom,
      y: (((cell.row - xterm.buffer.active.viewportY) * height) / xterm.rows - props.pan().y) / zoom,
    };
  };

  type Finger = { id: number; from: Point; word: Span | null };
  let finger: Finger | null = null;
  let holding: ReturnType<typeof setTimeout> | undefined;
  const onControls = (event: Event) =>
    event.target instanceof Element && event.target.closest(".hdl, .selmenu") !== null;
  const letGo = () => {
    clearTimeout(holding);
    finger = null;
    setMoving(false);
  };
  onCleanup(letGo);

  const attach = (pane: HTMLElement) => {
    pane.addEventListener(
      "touchstart",
      (event) => {
        if (onControls(event)) return;
        letGo();
        const touch = event.changedTouches[0];
        if (event.touches.length > 1 || touch === undefined) return;
        const held: Finger = { id: touch.identifier, from: { x: touch.clientX, y: touch.clientY }, word: null };
        finger = held;
        holding = setTimeout(() => {
          const anchor = cellAt(held.from, true);
          const cell = cellAt(held.from, false);
          const word = cell === null ? null : wordAt(xterm, cell);
          if (anchor === null || (word === null && !props.canPaste())) return;
          keepTouch();
          feel("lift");
          setSpot(word === null ? anchor : null);
          if (word === null) return;
          held.word = word;
          select(word);
          setMoving(true);
        }, LONG_PRESS_MS);
      },
      { passive: true },
    );
    pane.addEventListener(
      "touchmove",
      (event) => {
        const touch = fingerIn(event.touches, finger?.id);
        if (finger === null || touch === undefined) return;
        const to = { x: touch.clientX, y: touch.clientY };
        const { word } = finger;
        if (word === null) {
          if (Math.hypot(to.x - finger.from.x, to.y - finger.from.y) >= COMMIT_PX) letGo();
          return;
        }
        const cell = cellAt(to, true);
        if (cell === null) return;
        if (before(cell, word.from)) select({ from: cell, to: word.to });
        else if (before(word.to, cell)) select({ from: word.from, to: cell });
        else select(word);
      },
      { passive: true },
    );
    for (const lift of ["touchend", "touchcancel"] as const)
      pane.addEventListener(lift, (event) => {
        if (fingerIn(event.changedTouches, finger?.id) !== undefined) letGo();
      });
  };

  /** A handle takes its finger on touch-down and moves its own end, the other end staying put. */
  const handle = (end: "from" | "to") => (element: HTMLElement) => {
    let grip: { id: number; fixed: Cell; offset: Point } | null = null;
    element.addEventListener(
      "touchstart",
      (event) => {
        const touch = event.changedTouches[0];
        const held = span();
        if (touch === undefined || held === null) return;
        keepTouch();
        const middle = middleOf(held[end]);
        grip = {
          id: touch.identifier,
          fixed: end === "from" ? held.to : held.from,
          offset: { x: middle.x - touch.clientX, y: middle.y - touch.clientY },
        };
        setMoving(true);
      },
      { passive: true },
    );
    element.addEventListener(
      "touchmove",
      (event) => {
        const touch = fingerIn(event.touches, grip?.id);
        if (grip === null || touch === undefined) return;
        const cell = cellAt({ x: touch.clientX + grip.offset.x, y: touch.clientY + grip.offset.y }, true);
        if (cell !== null) select(ordered(grip.fixed, cell));
      },
      { passive: true },
    );
    for (const lift of ["touchend", "touchcancel"] as const)
      element.addEventListener(lift, (event) => {
        if (fingerIn(event.changedTouches, grip?.id) === undefined) return;
        grip = null;
        setMoving(false);
      });
  };

  const placed = (point: Point) => ({
    left: `calc(var(--g1) + ${point.x}px)`,
    top: `calc(var(--g1) + ${point.y}px)`,
  });
  /** Over the first line, else under the last, else over the selection; centered on one line, else on the pane. */
  const menuAt = (held: Span, size: Size | null) => {
    if (size === null) return { visibility: "hidden" as const };
    const top = cornerOf(held.from);
    const bottom = cornerOf({ col: held.to.col + 1, row: held.to.row + 1 });
    const zoom = scale(ui.prefs);
    const room = { width: props.view().width / zoom, height: props.view().height / zoom };
    const middle = held.from.row === held.to.row ? (top.x + bottom.x) / 2 : room.width / 2;
    const gap = px("--g2");
    const over = top.y - gap - size.height;
    const under = bottom.y + gap;
    const y = over >= 0 ? over : under + size.height <= room.height ? under : 0;
    return placed({ x: Math.min(Math.max(middle - size.width / 2, 0), room.width - size.width), y });
  };

  /**
   * The span's box in the OS's points: its one line, else its whole lines. The body's zoom scales the page, and WebKit
   * may report a box inside it before or after the zoom, the body's own box telling which.
   */
  const boxOf = (held: Span): Box => {
    const screen = props.screen()!.getBoundingClientRect();
    const cell = { width: screen.width / xterm.cols, height: screen.height / xterm.rows };
    const oneLine = held.from.row === held.to.row;
    const left = oneLine ? screen.left + held.from.col * cell.width : screen.left;
    const right = oneLine ? screen.left + (held.to.col + 1) * cell.width : screen.right;
    const top = screen.top + (held.from.row - xterm.buffer.active.viewportY) * cell.height;
    const points = window.innerWidth / document.body.getBoundingClientRect().width;
    const lines = held.to.row - held.from.row + 1;
    return { x: left * points, y: top * points, width: (right - left) * points, height: lines * cell.height * points };
  };

  /** The selection, else the spot while the menu has Paste to offer there. */
  const menuAnchor = () => {
    const at = spot();
    return span() ?? (at !== null && props.canPaste() ? { from: at, to: at } : null);
  };

  const Handles = () => (
    <>
      <Show when={span()}>
        {(held) => (
          <>
            <i class="hdl start" ref={handle("from")} style={placed(cornerOf(held().from))} />
            <i
              class="hdl end"
              ref={handle("to")}
              style={placed(cornerOf({ col: held().to.col + 1, row: held().to.row }))}
            />
          </>
        )}
      </Show>
      <Show when={!moving() && menuAnchor()}>
        {(at) => (
          <SelectionMenu
            selection={span() === null ? null : xterm.getSelection()}
            paste={props.canPaste()}
            place={(size) => menuAt(at(), size)}
            box={() => boxOf(at())}
            done={dismiss}
            pasted={props.paste}
          />
        )}
      </Show>
    </>
  );

  return {
    attach,
    Handles,
    /** Whether a touch landed on a handle or on the menu, which the terminal's own tap leaves alone. */
    onControls,
    cellAt,
    /** Clears a selection or the menu, or opens the URL under the tap; reports whether it did either. */
    tap: (point: Point) => {
      if (xterm.hasSelection() || spot() !== null) {
        dismiss();
        return true;
      }
      const cell = cellAt(point, false);
      const url = cell === null ? null : linkAt(xterm, cell);
      if (url === null) return false;
      props.open(url);
      return true;
    },
  };
}
