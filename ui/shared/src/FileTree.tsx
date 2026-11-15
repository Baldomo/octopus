import { batch, createEffect, createMemo, createSignal, For, onCleanup, Show } from "solid-js";
import { mac } from "./commands";
import type { EntryKind, TaskView } from "./generated";
import { keepInView } from "./keepInView";
import { onFileDrag } from "./fileDrag";
import { closeMenu, Head, Item, onRightClick, openMenu, Sep } from "./Menu";
import {
  absolute,
  copyPaths,
  create,
  deletion,
  duplicate,
  folderOf,
  landed,
  outermost,
  paste,
  place,
  remove,
  rename,
  subject,
  type Say,
} from "./entries";
import { said } from "./errors";
import { openShell } from "./panels";
import { toast } from "./Toast";
import { act, isLocal } from "./state";
import { join, parentOf, within, type TaskFiles } from "./taskFiles";

export type Outside = (task: TaskView, path: string, how: "reveal" | "open") => Promise<void>;

/** DESIGN's tree indents each level, and spaces the modified mark, with two non-breaking spaces. */
const GAP = "  ";
const indent = (depth: number) => GAP.repeat(depth);

const DRAG_PX = 4;
const EXPAND_MS = 600;

const KEYS = mac
  ? { cut: "⌘X", copy: "⌘C", paste: "⌘V", path: "⌥⌘C", relative: "⌥⇧⌘C", reveal: "⌥⌘R", delete: "⌘⌫" }
  : {
      cut: "Ctrl+X",
      copy: "Ctrl+C",
      paste: "Ctrl+V",
      path: "Ctrl+Alt+C",
      relative: "Ctrl+Alt+Shift+C",
      reveal: "Ctrl+Alt+R",
      delete: "Delete",
    };
const SAY: Say = { done: toast, problem: toast };
const REVEAL = mac ? "Reveal in Finder" : "Reveal in file manager";

type Row = { path: string; name: string; kind: EntryKind; depth: number };
type Draft = { kind: EntryKind; dir: string } | { rename: string };
/** Keyed by where it adds what, so a new draft gets a fresh field. */
const DRAFT = "\0draft";
const draftKey = (editing: { kind: EntryKind; dir: string }) => `${DRAFT}${editing.kind}:${editing.dir}`;

const [draft, setDraft] = createSignal<Draft | null>(null);
const [refusal, setRefusal] = createSignal<string | null>(null);
const [dragged, setDragged] = createSignal<string[] | null>(null);
const [target, setTarget] = createSignal<string | null>(null);

function startDraft(files: TaskFiles, next: Draft) {
  batch(() => {
    closeMenu();
    if ("dir" in next && next.dir !== ".") files.reveal(next.dir, true);
    setRefusal(null);
    setDraft(next);
  });
}

/** Deleting confirms in place. */
function ConfirmDelete(props: { files: TaskFiles; task: TaskView; paths: string[]; back: () => void }) {
  const asked = () => deletion(props.task, props.paths);
  const go = () => {
    closeMenu();
    remove(props.files, props.paths, asked().trash, SAY);
  };
  return (
    <div class="confirm">
      <span>{asked().question}</span>
      <div class="row">
        <button class="btn" onClick={() => props.back()}>
          Cancel
        </button>
        <button
          class="btn danger"
          ref={(button) => requestAnimationFrame(() => button.focus())}
          onKeyDown={(event) => event.key === "Enter" && event.stopPropagation()}
          onClick={go}
        >
          {asked().button}
        </button>
      </div>
    </div>
  );
}

type MenuProps = {
  files: TaskFiles;
  task: TaskView;
  outside?: Outside;
  /** The entry right-clicked, or `.` for the task's directory. */
  path: string;
  kind: EntryKind;
  paths: string[];
  deleting?: boolean;
};

function EntryMenu(props: MenuProps) {
  const [deleting, setDeleting] = createSignal(props.deleting === true);
  const root = props.path === ".";
  const dir = folderOf(props.path, props.kind);
  const local = () => props.outside !== undefined && isLocal(props.task.at.server);
  const closing = (action: () => unknown) => () => {
    closeMenu();
    action();
  };
  return (
    <Show
      when={!deleting()}
      fallback={
        <ConfirmDelete files={props.files} task={props.task} paths={props.paths} back={() => setDeleting(false)} />
      }
    >
      <Head
        title={root ? "Task directory" : subject(props.paths)}
        lines={[props.paths.length === 1 ? absolute(props.task, props.path) : props.paths.join(", ")]}
      />
      <Item run={() => startDraft(props.files, { kind: "file", dir })}>New file…</Item>
      <Item run={() => startDraft(props.files, { kind: "directory", dir })}>New folder…</Item>
      <Item
        why={props.task.live ? undefined : "task ended"}
        run={closing(() => act(openShell(props.task.at, props.task.key, dir)))}
      >
        Open in terminal
      </Item>
      <Show when={local()}>
        <Item kbd={KEYS.reveal} run={closing(() => act(props.outside!(props.task, props.path, "reveal")))}>
          {REVEAL}
        </Item>
        <Show when={props.kind === "file"}>
          <Item run={closing(() => act(props.outside!(props.task, props.path, "open")))}>Open in default app</Item>
        </Show>
      </Show>
      <Sep />
      <Show when={!root}>
        <Item kbd={KEYS.cut} run={closing(() => props.files.setClip({ paths: props.paths, cut: true }))}>
          Cut
        </Item>
        <Item kbd={KEYS.copy} run={closing(() => props.files.setClip({ paths: props.paths, cut: false }))}>
          Copy
        </Item>
      </Show>
      <Item
        kbd={KEYS.paste}
        why={props.files.state.clip === null ? "nothing copied" : undefined}
        run={closing(() => paste(props.files, dir, SAY))}
      >
        Paste
      </Item>
      <Show when={!root}>
        <Item run={closing(() => duplicate(props.files, props.paths, SAY))}>Duplicate</Item>
      </Show>
      <Sep />
      <Item kbd={KEYS.path} run={closing(() => copyPaths(props.task, props.paths, true, SAY))}>
        Copy path
      </Item>
      <Show when={!root}>
        <Item kbd={KEYS.relative} run={closing(() => copyPaths(props.task, props.paths, false, SAY))}>
          Copy relative path
        </Item>
        <Sep />
        <Item
          kbd="F2"
          why={props.paths.length > 1 ? "one at a time" : undefined}
          run={() => startDraft(props.files, { rename: props.path })}
        >
          Rename…
        </Item>
        <Item danger kbd={KEYS.delete} run={() => setDeleting(true)}>
          Delete…
        </Item>
      </Show>
      <Show when={root}>
        <Sep />
        <Item run={closing(() => props.files.collapseAll())}>Collapse all folders</Item>
      </Show>
    </Show>
  );
}

function NameField(props: { value: string; apply: (name: string) => Promise<unknown> }) {
  const stop = (event: Event) => event.stopPropagation();
  const stem = props.value.lastIndexOf(".") > 0 ? props.value.lastIndexOf(".") : props.value.length;
  return (
    <input
      class="rename"
      spellcheck={false}
      value={props.value}
      ref={(input) =>
        requestAnimationFrame(() => {
          input.focus();
          input.setSelectionRange(0, stem);
        })
      }
      onPointerDown={stop}
      onClick={stop}
      onContextMenu={stop}
      onInput={() => setRefusal(null)}
      onKeyDown={(event) => {
        stop(event);
        if (event.key === "Escape") setDraft(null);
        if (event.key !== "Enter") return;
        const name = event.currentTarget.value.trim();
        if (name === "" || name === props.value) return setDraft(null);
        props.apply(name).then(
          () => setDraft(null),
          (error) => setRefusal(said(error)),
        );
      }}
      onBlur={() => setDraft(null)}
    />
  );
}

export function FileTree(props: { files: TaskFiles; task: TaskView; outside?: Outside }) {
  let tree!: HTMLDivElement;
  const files = () => props.files;
  const state = () => props.files.state;

  const shown = createMemo(() => {
    const rows = new Map<string, Row>();
    const order: string[] = [];
    const editing = draft();
    const walk = (dir: string, depth: number) => {
      if (editing !== null && "dir" in editing && editing.dir === dir) {
        order.push(draftKey(editing));
        rows.set(draftKey(editing), { path: editing.dir, name: "", kind: editing.kind, depth });
      }
      for (const entry of state().listings[dir] ?? []) {
        const path = join(dir, entry.name);
        rows.set(path, { path, name: entry.name, kind: entry.kind, depth });
        order.push(path);
        if (entry.kind === "directory" && state().expanded[path]) walk(path, depth + 1);
      }
    };
    walk(".", 0);
    return { rows, order, paths: order.filter((path) => !path.startsWith(DRAFT)) };
  });

  const rowElement = (path: string) => tree.querySelector<HTMLElement>(`[data-path="${CSS.escape(path)}"]`);
  const kindOf = (path: string) => shown().rows.get(path)?.kind ?? "directory";
  const coveredBy = (path: string) => (state().selected.includes(path) ? state().selected : [path]);

  const menu =
    (path: string, deleting = false) =>
    () => (
      <EntryMenu
        files={files()}
        task={props.task}
        outside={props.outside}
        path={path}
        kind={path === "." ? "directory" : kindOf(path)}
        paths={path === "." ? ["."] : coveredBy(path)}
        deleting={deleting}
      />
    );

  const activate = (row: Row) => {
    if (row.kind === "directory") files().toggle(row.path);
    else act(files().open(row.path));
  };

  const range = (from: string, to: string) => {
    const { paths } = shown();
    const [start, end] = [paths.indexOf(from), paths.indexOf(to)];
    return start < 0 ? [to] : paths.slice(Math.min(start, end), Math.max(start, end) + 1);
  };

  const click = (row: Row, event: MouseEvent) => {
    tree.focus();
    const anchor = state().anchor ?? row.path;
    if (event.shiftKey) return files().select(range(anchor, row.path), row.path, anchor);
    if (mac ? event.metaKey : event.ctrlKey) {
      const selected = state().selected;
      return files().select(
        selected.includes(row.path) ? selected.filter((path) => path !== row.path) : [...selected, row.path],
        row.path,
      );
    }
    files().select([row.path], row.path);
    activate(row);
  };

  const rightClick = (row: Row, event: MouseEvent) => {
    if (!state().selected.includes(row.path)) files().select([row.path], row.path);
    onRightClick(`file:${row.path}`, () => rowElement(row.path) ?? tree, menu(row.path))(event);
  };

  const dropDirAt = (x: number, y: number) =>
    document.elementFromPoint(x, y)?.closest<HTMLElement>("[data-drop-dir]")?.dataset.dropDir ?? null;

  /** A move needs somewhere new to go, and a folder never goes inside itself. */
  const takes = (dir: string, paths: string[], copying: boolean) =>
    paths.every((path) => !within(dir, path)) && (copying || paths.some((path) => parentOf(path) !== dir));

  let expandTimer: ReturnType<typeof setTimeout> | undefined;
  let endDrag: (() => unknown) | undefined;
  const aim = (dir: string | null) => {
    if (dir === target()) return;
    clearTimeout(expandTimer);
    setTarget(dir);
    if (dir !== null && dir !== "." && !state().expanded[dir])
      expandTimer = setTimeout(() => target() === dir && !state().expanded[dir] && files().toggle(dir), EXPAND_MS);
  };

  /** Drags run on pointer events, as the sidebar's do; ⌥ at the drop copies instead of moving. */
  const press = (row: Row, down: PointerEvent) => {
    if (down.button !== 0 || draft() !== null) return;
    const move = (event: PointerEvent) => {
      if (dragged() === null) {
        if (Math.hypot(event.clientX - down.clientX, event.clientY - down.clientY) < DRAG_PX) return;
        if (!state().selected.includes(row.path)) files().select([row.path], row.path);
        setDragged(outermost(state().selected));
      }
      const dir = dropDirAt(event.clientX, event.clientY);
      aim(dir !== null && takes(dir, dragged()!, event.altKey) ? dir : null);
    };
    const end = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", end);
      window.removeEventListener("keydown", escape, { capture: true });
      clearTimeout(expandTimer);
      endDrag = undefined;
      const [paths, dir] = [dragged(), target()];
      batch(() => {
        setDragged(null);
        setTarget(null);
      });
      return { paths, dir };
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.stopPropagation();
      end();
    };
    const up = (event: PointerEvent) => {
      const { paths, dir } = end();
      if (paths === null) return;
      const swallow = (click: MouseEvent) => click.stopPropagation();
      window.addEventListener("click", swallow, { capture: true, once: true });
      setTimeout(() => window.removeEventListener("click", swallow, { capture: true }));
      if (dir !== null) place(files(), paths, dir, event.altKey, SAY);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", end);
    endDrag = end;
    window.addEventListener("keydown", escape, { capture: true });
  };

  onFileDrag((drag, under) => {
    const dir =
      drag.kind !== "leave" && under !== null && tree.contains(under) ? (dropDirAt(drag.x, drag.y) ?? ".") : null;
    aim(drag.kind === "over" ? dir : null);
    if (drag.kind === "drop" && dir !== null)
      act(
        files()
          .bringIn(drag.paths, dir, toast)
          .then((copies) => landed(files(), dir, copies.length, "Copied", SAY)),
      );
  });

  const keys = (event: KeyboardEvent) => {
    if (draft() !== null || event.target !== tree) return;
    const { paths, rows } = shown();
    const focus = state().focus !== null && rows.has(state().focus!) ? state().focus! : null;
    const row = focus === null ? undefined : rows.get(focus);
    const at = focus === null ? -1 : paths.indexOf(focus);
    const chosen = focus === null ? [] : coveredBy(focus);
    const go = (to: string | undefined) => {
      if (to === undefined) return;
      const anchor = state().anchor ?? to;
      if (event.shiftKey) files().select(range(anchor, to), to, anchor);
      else files().select([to], to);
    };
    const clip = (cut: boolean) => chosen.length > 0 && files().setClip({ paths: chosen, cut });
    const copyPath = (full: boolean) => chosen.length > 0 && copyPaths(props.task, chosen, full, SAY);
    const open = () => row !== undefined && activate(row);
    const remove = () =>
      row !== undefined && openMenu(`file:${row.path}`, rowElement(row.path) ?? tree, menu(row.path, true));
    const rename = () => row !== undefined && startDraft(files(), { rename: row.path });
    const bindings: Record<string, () => unknown> = {
      "mod+KeyC": () => clip(false),
      "mod+KeyX": () => clip(true),
      "mod+KeyV": () => paste(files(), row === undefined ? "." : folderOf(row.path, row.kind), SAY),
      "mod+KeyA": () => files().select(paths, focus, paths[0] ?? null),
      "mod+alt+KeyC": () => copyPath(true),
      "mod+alt+shift+KeyC": () => copyPath(false),
      "mod+alt+KeyR": () =>
        row !== undefined &&
        props.outside !== undefined &&
        isLocal(props.task.at.server) &&
        act(props.outside(props.task, row.path, "reveal")),
      "mod+ArrowDown": open,
      "mod+Backspace": remove,
      Delete: remove,
      Space: open,
      Enter: rename,
      F2: rename,
      ArrowDown: () => go(paths[Math.min(at + 1, paths.length - 1)]),
      ArrowUp: () => go(paths[Math.max(at - 1, 0)]),
      Home: () => go(paths[0]),
      End: () => go(paths.at(-1)),
      ArrowRight: () =>
        row?.kind === "directory" && (state().expanded[row.path] ? go(paths[at + 1]) : files().toggle(row.path)),
      ArrowLeft: () => {
        if (row === undefined) return;
        if (row.kind === "directory" && state().expanded[row.path]) files().toggle(row.path);
        else if (parentOf(row.path) !== ".") go(parentOf(row.path));
      },
      Escape: () => (state().clip?.cut ? files().setClip(null) : focus !== null && files().select([focus], focus)),
    };
    const extending = ["ArrowDown", "ArrowUp", "Home", "End"].includes(event.code) && event.shiftKey;
    const chord = [
      (mac ? event.metaKey : event.ctrlKey) && "mod",
      (mac ? event.ctrlKey : event.metaKey) && "other",
      event.altKey && "alt",
      event.shiftKey && !extending && "shift",
      event.code,
    ]
      .filter(Boolean)
      .join("+");
    const run = bindings[chord];
    if (run === undefined) return;
    event.preventDefault();
    run();
  };

  createEffect(() => {
    const revealing = state().revealing;
    if (revealing === null || !shown().rows.has(revealing)) return;
    rowElement(revealing)?.scrollIntoView({ block: "nearest" });
    files().revealed();
  });
  onCleanup(() => {
    endDrag?.();
    clearTimeout(expandTimer);
    batch(() => {
      setDraft(null);
      setTarget(null);
    });
  });

  const inTarget = (path: string) => target() !== null && target() !== "." && within(path, target()!);

  return (
    <div
      ref={tree}
      class="tree"
      classList={{ over: target() === "." }}
      tabIndex={0}
      data-drop-dir="."
      onKeyDown={keys}
      onContextMenu={(event) => {
        files().select([], null);
        onRightClick("file:.", () => tree, menu("."))(event);
      }}
    >
      <For each={shown().order}>
        {(path) => {
          const first = shown().rows.get(path)!;
          const row = () => shown().rows.get(path) ?? first;
          if (path.startsWith(DRAFT)) {
            const editing = { kind: first.kind, dir: first.path };
            return (
              <>
                <div class="draft">
                  {indent(row().depth)}
                  {row().kind === "directory" ? "▸ " : ""}
                  <NameField value="" apply={(name) => create(files(), editing.dir, name, editing.kind)} />
                </div>
                <Show when={refusal()}>{(why) => <div class="err">{why()}</div>}</Show>
              </>
            );
          }
          const renaming = () => {
            const editing = draft();
            return editing !== null && "rename" in editing && editing.rename === path;
          };
          const directory = () => row().kind === "directory";
          return (
            <>
              <div
                data-path={path}
                data-drop-dir={folderOf(path, row().kind)}
                title={path}
                ref={keepInView(() => state().focus === path)}
                classList={{
                  dir: directory(),
                  on: state().active === path,
                  mod: state().dirty[path] === true,
                  sel: state().selected.includes(path),
                  focus: state().focus === path,
                  cut: files().isCut(path),
                  dragging: dragged()?.some((moving) => within(path, moving)) === true,
                  over: target() === path,
                  within: inTarget(path) && target() !== path,
                }}
                onPointerDown={(event) => press(row(), event)}
                onClick={(event) => click(row(), event)}
                onContextMenu={(event) => rightClick(row(), event)}
              >
                {indent(row().depth)}
                {directory() ? (state().expanded[path] ? "▾ " : "▸ ") : ""}
                <Show when={renaming()} fallback={row().name}>
                  <NameField value={row().name} apply={(name) => rename(files(), path, name)} />
                </Show>
                {state().dirty[path] ? `${GAP}M` : ""}
              </div>
              <Show when={renaming() && refusal()}>{(why) => <div class="err">{why()}</div>}</Show>
            </>
          );
        }}
      </For>
    </div>
  );
}
