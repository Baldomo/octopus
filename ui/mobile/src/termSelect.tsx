import type { Terminal } from "@xterm/xterm";
import { createSignal, onCleanup, Show } from "solid-js";
import { report, scale, ui } from "@octopus/shared/state.ts";
import { feel } from "./feel";
import { phone } from "./phone";
import { LONG_PRESS_MS, type Point } from "./press";
import { notify } from "./Toast";
import { COMMIT_PX, keepTouch } from "./touch";

/** A buffer cell; `row` counts from the first line of the scrollback. */
type Cell = { col: number; row: number };
/** Both ends inclusive, `from` first. */
type Span = { from: Cell; to: Cell };
type Size = { width: number; height: number };
/** One character of a wrapped line and the cell it starts at; a wide character takes two cells. */
type Char = { text: string; cell: Cell; width: number };

/** xterm's double-click word separators, so a word reads the same on the phone and the desktop. */
const SEPARATORS = " ()[]{}',\"`";
const LINK = /https?:\/\/[^\s<>"'`]+/g;
const SENTENCE_END = /[.,;:!?]$/;
const OPENER: Record<string, string> = { ")": "(", "]": "[", "}": "{" };
const MENU_GAP_PT = 8;

const before = (a: Cell, b: Cell) => a.row < b.row || (a.row === b.row && a.col < b.col);
const ordered = (a: Cell, b: Cell): Span => (before(b, a) ? { from: b, to: a } : { from: a, to: b });
const count = (text: string, char: string) => text.split(char).length - 1;

function wrappedLine(xterm: Terminal, row: number): Char[] {
  const buffer = xterm.buffer.active;
  let first = row;
  while (first > 0 && buffer.getLine(first)?.isWrapped) first--;
  let last = row;
  while (buffer.getLine(last + 1)?.isWrapped) last++;
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

const charAt = (chars: Char[], cell: Cell) => chars.findLastIndex((char) => !before(cell, char.cell));
const lastCell = (char: Char): Cell => ({ col: char.cell.col + char.width - 1, row: char.cell.row });

/** The word covering `cell`, or the one character when it is a separator. */
function wordAt(xterm: Terminal, cell: Cell): Span {
  const chars = wrappedLine(xterm, cell.row);
  const inWord = (nth: number) => chars[nth] !== undefined && !SEPARATORS.includes(chars[nth].text);
  let first = charAt(chars, cell);
  let last = first;
  if (inWord(first)) {
    while (inWord(first - 1)) first--;
    while (inWord(last + 1)) last++;
  }
  const [start, end] = [chars[first], chars[last]];
  return start && end ? { from: start.cell, to: lastCell(end) } : { from: cell, to: cell };
}

/** A URL less the punctuation closing the sentence around it, keeping a bracket the URL opened. */
function trimmed(url: string): string {
  for (;;) {
    const end = url.at(-1) ?? "";
    const opener = OPENER[end];
    const closes = opener !== undefined && count(url, end) > count(url, opener);
    if (!SENTENCE_END.test(end) && !closes) return url;
    url = url.slice(0, -1);
  }
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
  for (const match of text.matchAll(LINK)) {
    const url = trimmed(match[0]);
    if (offset >= match.index && offset < match.index + url.length) return url;
  }
  return null;
}

/**
 * Touch selection over xterm's selection API, which xterm itself drives only by mouse. A finger maps to a cell through
 * the drawn screen's box on the page; handles and Copy sit in the pane's points, xterm's pixels divided by the zoom.
 */
export function termSelection(props: {
  xterm: Terminal;
  /** The drawn screen, wherever the pan has moved it; absent until the terminal first paints. */
  screen: () => HTMLElement | undefined;
  /** The drawn screen's size, and the part of it the pane shows, in xterm's pixels. */
  size: () => Size;
  view: () => Size;
  pan: () => Point;
}) {
  const { xterm } = props;
  /** Read again on every frame xterm draws, since scrolling and trimmed scrollback move it without a change event. */
  const [span, setSpan] = createSignal<Span | null>(null, { equals: false });
  /** A finger making or moving the selection keeps Copy out of its way. */
  const [moving, setMoving] = createSignal(false);

  const readSelection = () => {
    const at = xterm.getSelectionPosition();
    setSpan(
      at === undefined
        ? null
        : { from: { col: at.start.x, row: at.start.y }, to: { col: at.end.x - 1, row: at.end.y } },
    );
  };
  const select = ({ from, to }: Span) =>
    xterm.select(from.col, from.row, (to.row - from.row) * xterm.cols + to.col - from.col + 1);
  const listeners = [
    xterm.onSelectionChange(readSelection),
    xterm.onRender(readSelection),
    xterm.onResize(() => xterm.clearSelection()),
  ];
  onCleanup(() => listeners.forEach((listener) => listener.dispose()));

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

  /** A finger on the pane, and the word it selected once held past the long press. */
  type Finger = { id: number; from: Point; word: Span | null };
  let finger: Finger | null = null;
  let holding: ReturnType<typeof setTimeout> | undefined;
  const touchOf = (touches: TouchList, id: number | undefined) => [...touches].find((touch) => touch.identifier === id);
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
          const cell = cellAt(held.from, false);
          if (cell === null) return;
          keepTouch();
          feel("lift");
          held.word = wordAt(xterm, cell);
          select(held.word);
          setMoving(true);
        }, LONG_PRESS_MS);
      },
      { passive: true },
    );
    pane.addEventListener(
      "touchmove",
      (event) => {
        const touch = touchOf(event.touches, finger?.id);
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
        if (touchOf(event.changedTouches, finger?.id) !== undefined) letGo();
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
        const touch = touchOf(event.touches, grip?.id);
        if (grip === null || touch === undefined) return;
        const cell = cellAt({ x: touch.clientX + grip.offset.x, y: touch.clientY + grip.offset.y }, true);
        if (cell !== null) select(ordered(grip.fixed, cell));
      },
      { passive: true },
    );
    for (const lift of ["touchend", "touchcancel"] as const)
      element.addEventListener(lift, (event) => {
        if (touchOf(event.changedTouches, grip?.id) === undefined) return;
        grip = null;
        setMoving(false);
      });
  };

  const copy = () =>
    navigator.clipboard.writeText(xterm.getSelection()).then(() => {
      xterm.clearSelection();
      notify("Copied the selection");
    }, report);

  const placed = (point: Point) => ({
    left: `calc(var(--rail) + ${point.x}px)`,
    top: `calc(var(--g2) + ${point.y}px)`,
  });
  const [menuSize, setMenuSize] = createSignal<Size | null>(null);
  const measureMenu = (menu: HTMLElement) => {
    const observer = new ResizeObserver(() => setMenuSize({ width: menu.offsetWidth, height: menu.offsetHeight }));
    observer.observe(menu);
    onCleanup(() => observer.disconnect());
  };
  /** Over the first line, else under the last, else over the selection; centered on one line, else on the pane. */
  const menuAt = (held: Span) => {
    const size = menuSize();
    if (size === null) return { visibility: "hidden" as const };
    const top = cornerOf(held.from);
    const bottom = cornerOf({ col: held.to.col + 1, row: held.to.row + 1 });
    const zoom = scale(ui.prefs);
    const room = { width: props.view().width / zoom, height: props.view().height / zoom };
    const middle = held.from.row === held.to.row ? (top.x + bottom.x) / 2 : room.width / 2;
    const over = top.y - MENU_GAP_PT - size.height;
    const under = bottom.y + MENU_GAP_PT;
    const y = over >= 0 ? over : under + size.height <= room.height ? under : 0;
    return placed({ x: Math.min(Math.max(middle - size.width / 2, 0), room.width - size.width), y });
  };

  const Handles = () => (
    <Show when={span()}>
      {(held) => (
        <>
          <i class="hdl start" ref={handle("from")} style={placed(cornerOf(held().from))} />
          <i
            class="hdl end"
            ref={handle("to")}
            style={placed(cornerOf({ col: held().to.col + 1, row: held().to.row }))}
          />
          <Show when={!moving()}>
            <button ref={measureMenu} class="selmenu" style={menuAt(held())} onClick={copy}>
              Copy
            </button>
          </Show>
        </>
      )}
    </Show>
  );

  return {
    attach,
    Handles,
    /** Whether a touch landed on a handle or on Copy, which the terminal's own tap leaves alone. */
    onControls,
    /** The cell under a point on the drawn screen, or null off it. */
    cellAt: (point: Point) => cellAt(point, false),
    /** Clears a selection, or opens the URL under the tap; reports whether it did either. */
    tap: (point: Point) => {
      if (xterm.hasSelection()) {
        xterm.clearSelection();
        return true;
      }
      const cell = cellAt(point, false);
      const url = cell === null ? null : linkAt(xterm, cell);
      if (url === null) return false;
      phone.open(url).catch(report);
      return true;
    },
  };
}
