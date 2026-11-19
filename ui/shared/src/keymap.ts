import type { Terminal } from "@xterm/xterm";

export type Platform = "mac" | "linux";
/** A press as `[ctrl+][alt+][meta+][shift+]<KeyboardEvent.key>`. */
type Chord = string;

const GLYPHS = { meta: "⌘", alt: "⌥", ArrowLeft: "←", ArrowRight: "→", Backspace: "⌫", Delete: "⌦" } as const;
type MacEditKey = `${"meta" | "alt"}+${"ArrowLeft" | "ArrowRight" | "Backspace" | "Delete"}`;
export type Edit = { bytes: string; mac: MacEditKey; linux: Chord | null };

/** The line-editing actions as the readline and zle keys every shell and agent prompt reads, and each platform's key. */
export const EDITS: Edit[] = [
  { bytes: "\x01", mac: "meta+ArrowLeft", linux: "Home" },
  { bytes: "\x1bb", mac: "alt+ArrowLeft", linux: "ctrl+ArrowLeft" },
  { bytes: "\x1bf", mac: "alt+ArrowRight", linux: "ctrl+ArrowRight" },
  { bytes: "\x05", mac: "meta+ArrowRight", linux: "End" },
  { bytes: "\x1b\x7f", mac: "alt+Backspace", linux: "ctrl+Backspace" },
  { bytes: "\x1bd", mac: "alt+Delete", linux: "ctrl+Delete" },
  { bytes: "\x15", mac: "meta+Backspace", linux: null },
];

export type KeyAction = { write: string } | { clipboard: "copy" | "paste" };
/**
 * Keys that act even while the program reads exact keys, since xterm.js has no exact encoding to give it. Shift+Enter
 * writes ESC CR, which agents read as a newline apart from Enter.
 */
const APP_KEYS: { action: KeyAction; mac: Chord | null; linux: Chord }[] = [
  { action: { write: "\x1b\r" }, mac: "shift+Enter", linux: "shift+Enter" },
  { action: { clipboard: "copy" }, mac: null, linux: "ctrl+shift+C" },
  { action: { clipboard: "paste" }, mac: null, linux: "ctrl+shift+V" },
];

export const keyCap = (edit: Edit) => {
  const [modifier, key] = edit.mac.split("+") as [keyof typeof GLYPHS, keyof typeof GLYPHS];
  return GLYPHS[modifier] + GLYPHS[key];
};

type Press = Pick<KeyboardEvent, "ctrlKey" | "altKey" | "metaKey" | "shiftKey" | "key">;

const chord = (press: Press): Chord =>
  [press.ctrlKey && "ctrl", press.altKey && "alt", press.metaKey && "meta", press.shiftKey && "shift", press.key]
    .filter(Boolean)
    .join("+");

/** The action a press takes on `platform`; edits pass through unmapped while the program reads exact keys. */
export function keyAction(press: Press, platform: Platform, exactKeys: boolean): KeyAction | undefined {
  const pressed = chord(press);
  const bound = (each: { mac: Chord | null; linux: Chord | null }) => each[platform] === pressed;
  const edit = exactKeys ? undefined : EDITS.find(bound);
  return APP_KEYS.find(bound)?.action ?? (edit && { write: edit.bytes });
}

/**
 * Whether the program reads exact keys: it has pushed kitty keyboard flags on the screen it shows, or turned on xterm's
 * modifyOtherKeys at level 2. xterm.js speaks neither, so this reads the program's own requests as they pass, and `reset`
 * forgets them with the terminal's reset.
 */
export function exactKeys(
  parser: Pick<Terminal["parser"], "registerCsiHandler" | "registerEscHandler">,
  screen: () => "normal" | "alternate",
) {
  const kitty = { normal: [] as number[], alternate: [] as number[] };
  let modifyOtherKeys = false;
  const param = (params: (number | number[])[], index: number) =>
    typeof params[index] === "number" ? params[index] : 0;
  const heard = (change: (params: (number | number[])[]) => void) => (params: (number | number[])[]) => {
    change(params);
    return false;
  };
  parser.registerCsiHandler(
    { prefix: ">", final: "u" },
    heard((params) => kitty[screen()].push(param(params, 0))),
  );
  parser.registerCsiHandler(
    { prefix: "<", final: "u" },
    heard((params) => kitty[screen()].splice(-Math.max(param(params, 0), 1))),
  );
  parser.registerCsiHandler(
    { prefix: "=", final: "u" },
    heard((params) => {
      const stack = kitty[screen()];
      const [flags, mode] = [param(params, 0), param(params, 1) || 1];
      const top = stack.pop() ?? 0;
      stack.push(mode === 2 ? top | flags : mode === 3 ? top & ~flags : flags);
    }),
  );
  parser.registerCsiHandler(
    { prefix: ">", final: "m" },
    heard((params) => {
      if (param(params, 0) === 4) modifyOtherKeys = param(params, 1) === 2;
    }),
  );
  parser.registerCsiHandler(
    { prefix: ">", final: "n" },
    heard((params) => {
      if (param(params, 0) === 4) modifyOtherKeys = false;
    }),
  );
  const reset = () => {
    kitty.normal = [];
    kitty.alternate = [];
    modifyOtherKeys = false;
  };
  parser.registerEscHandler({ final: "c" }, () => {
    reset();
    return false;
  });
  return { on: () => modifyOtherKeys || (kitty[screen()].at(-1) ?? 0) !== 0, reset };
}
