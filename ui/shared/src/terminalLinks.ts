import type { IBufferRange, ILink, ILinkDecorations, Terminal } from "@xterm/xterm";
import { mac } from "./commands";

/** A ⌘-clicked link opens in the task's browser panel, and a ⌘⇧-clicked one in the system's browser. */
export type LinkTarget = "task" | "system";
export type OpenUrl = (url: string, target: LinkTarget) => void;

/** From `http://` or `https://` to whitespace, quotes or angle brackets; parentheses count only in balanced pairs. */
const URL_PATTERN = /https?:\/\/(?:[^\s"'<>`()]|\([^\s"'<>`()]*\))*(?:[^\s"'<>`().,:;!?*~\]}]|\([^\s"'<>`()]*\))/g;

/** ⌘ on macOS and Ctrl elsewhere turns a URL into a link. */
const linkKey = (event: KeyboardEvent | MouseEvent) => (mac ? event.metaKey : event.ctrlKey);
let held = false;
type UrlLink = ILink & { decorations: ILinkDecorations };
let hovered: UrlLink | undefined;
/** Until xterm first hovers a link it reads `held` live; from then on xterm's own decorations take each change. */
const whileHeld = (): ILinkDecorations => ({
  get underline() {
    return held;
  },
  set underline(_) {},
  get pointerCursor() {
    return held;
  },
  set pointerCursor(_) {},
});
const decorate = ({ decorations }: UrlLink) => Object.assign(decorations, { underline: held, pointerCursor: held });
const track = (holding: boolean) => {
  if (holding === held) return;
  held = holding;
  if (hovered) decorate(hovered);
};
for (const type of ["keydown", "keyup", "mousemove", "mousedown"] as const)
  addEventListener(type, (event) => track(linkKey(event)), true);
addEventListener("blur", () => track(false));

type Cell = { x: number; y: number; width: number };

/** The text of the row `y` (1-based) wraps into, with the cell each of its UTF-16 units sits in. */
function wrappedRow(xterm: Terminal, y: number) {
  const buffer = xterm.buffer.active;
  const cell = buffer.getNullCell();
  let top = y - 1;
  while (top > 0 && buffer.getLine(top)?.isWrapped) top--;
  let text = "";
  const cells: Cell[] = [];
  for (let row = top; ; row++) {
    const line = buffer.getLine(row);
    if (line === undefined || (row > top && !line.isWrapped)) break;
    for (let x = 0; x < line.length; x++) {
      line.getCell(x, cell);
      const width = cell.getWidth();
      if (width === 0) continue;
      const chars = cell.getChars() || " ";
      text += chars;
      for (let unit = 0; unit < chars.length; unit++) cells.push({ x: x + 1, y: row + 1, width });
    }
  }
  return { text, cells };
}

function urlsOn(xterm: Terminal, y: number) {
  const { text, cells } = wrappedRow(xterm, y);
  return [...text.matchAll(URL_PATTERN)].flatMap((match) => {
    const start = cells[match.index]!;
    const end = cells[match.index + match[0].length - 1]!;
    if (start.y > y || end.y < y) return [];
    const range: IBufferRange = { start: { x: start.x, y: start.y }, end: { x: end.x + end.width - 1, y: end.y } };
    return [{ url: match[0], range }];
  });
}

/** URLs the terminal prints, and its OSC 8 hyperlinks, open on a click with the link key held. */
export function openLinks(xterm: Terminal, open: OpenUrl) {
  const activate = (event: MouseEvent, url: string) => {
    if (linkKey(event)) open(url, event.shiftKey ? "system" : "task");
  };
  xterm.options.linkHandler = { activate };
  xterm.registerLinkProvider({
    provideLinks: (y, reply) =>
      reply(
        urlsOn(xterm, y).map(({ url, range }) => {
          const link: UrlLink = {
            text: url,
            range,
            decorations: whileHeld(),
            activate,
            hover: () => {
              hovered = link;
              decorate(link);
            },
            leave: () => {
              if (hovered === link) hovered = undefined;
            },
          };
          return link;
        }),
      ),
  });
}
