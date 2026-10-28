import { createEffect, createMemo, For, lazy, on, Show } from "solid-js";
import { keepInView } from "./keepInView";
import { commands, hint, mac } from "./commands";
import { filesOf, join, loadMonaco, nameOf, renderable, type TaskFiles } from "./taskFiles";
import { act, openView, report } from "./state";

type Editor = import("monaco-editor").editor.IStandaloneCodeEditor;
type Shown = { files: TaskFiles; path: string } | undefined;

/** DESIGN's tree indents each level, and spaces the modified mark, with two non-breaking spaces. */
const GAP = "  ";
const indent = (depth: number) => GAP.repeat(depth);

function Tree(props: { files: TaskFiles; dir: string; depth: number }) {
  const scrollWhenRevealed = (path: string) => (row: HTMLElement) =>
    createEffect(() => {
      if (props.files.state.revealing !== path) return;
      row.scrollIntoView({ block: "nearest" });
      props.files.revealed();
    });
  return (
    <For each={props.files.state.listings[props.dir] ?? []}>
      {(entry) => {
        const path = join(props.dir, entry.name);
        if (entry.kind === "directory") {
          const expanded = () => props.files.state.expanded[path] === true;
          return (
            <>
              <div class="dir" title={path} ref={scrollWhenRevealed(path)} onClick={() => props.files.toggle(path)}>
                {indent(props.depth)}
                {expanded() ? "▾" : "▸"} {entry.name}
              </div>
              <Show when={expanded()}>
                <Tree files={props.files} dir={path} depth={props.depth + 1} />
              </Show>
            </>
          );
        }
        const dirty = () => props.files.state.dirty[path] === true;
        return (
          <div
            title={path}
            ref={scrollWhenRevealed(path)}
            classList={{ on: props.files.state.active === path, mod: dirty() }}
            onClick={() => act(props.files.open(path))}
          >
            {indent(props.depth)}
            {entry.name}
            {dirty() ? `${GAP}M` : ""}
          </div>
        );
      }}
    </For>
  );
}

const dirsOf = (path: string) => path.split("/").slice(0, -1);
const tail = (dirs: string[], depth: number) => dirs.slice(-depth).join("/");

/** A tab names its file, and the fewest trailing directories that tell it from another open tab of the same name. */
function EditorTabs(props: { files: TaskFiles; visible: boolean }) {
  let bar!: HTMLDivElement;
  const where = createMemo(() => {
    const tabs = props.files.state.tabs;
    const hints: Record<string, string> = {};
    for (const path of tabs) {
      const rivals = tabs.filter((tab) => tab !== path && nameOf(tab) === nameOf(path)).map(dirsOf);
      if (rivals.length === 0) continue;
      const dirs = dirsOf(path);
      let depth = 1;
      while (depth < dirs.length && rivals.some((rival) => tail(rival, depth) === tail(dirs, depth))) depth++;
      hints[path] = dirs.length === 0 ? "." : tail(dirs, depth);
    }
    return hints;
  });
  const scrollSideways = (event: WheelEvent) => {
    if (Math.abs(event.deltaY) <= Math.abs(event.deltaX)) return;
    bar.scrollLeft += event.deltaY;
    event.preventDefault();
  };
  return (
    <div class="etabs" ref={bar} onWheel={scrollSideways}>
      <For each={props.files.state.tabs}>
        {(path) => {
          const on = () => props.files.state.active === path;
          const dirty = () => props.files.state.dirty[path] === true;
          return (
            <span
              ref={keepInView(() => props.visible && on())}
              classList={{ on: on(), mod: dirty() }}
              title={path}
              onClick={() => act(props.files.open(path))}
              onAuxClick={(event) => event.button === 1 && props.files.close(path)}
            >
              {nameOf(path)}
              <Show when={where()[path]}>{(hint) => <span class="where">{hint()}</span>}</Show>
              <span
                class="close"
                onClick={(event) => {
                  event.stopPropagation();
                  props.files.close(path);
                }}
              />
            </span>
          );
        }}
      </For>
    </div>
  );
}

/** The renderers load on the first rendered view, off the launch path. */
const Rendered = lazy(() => import("./Rendered").then(({ Rendered }) => ({ default: Rendered })));

const EYE = (
  <svg viewBox="0 0 24 24">
    <path d="M2 12s3.6-7 10-7 10 7 10 7-3.6 7-10 7S2 12 2 12z" />
    <circle cx="12" cy="12" r="3" />
  </svg>
);
const CODE = (
  <svg viewBox="0 0 24 24">
    <path d="m16 18 6-6-6-6" />
    <path d="m8 6-6 6 6 6" />
  </svg>
);

/** The selected file's path, each part revealing itself in the tree, and the switch between its code and its render. */
function FileHead(props: { files: TaskFiles; path: string }) {
  const parts = () => props.path.split("/");
  const rendered = () => props.files.state.rendered[props.path] === true;
  return (
    <div class="fhead">
      <span class="path">
        <span>
          <For each={parts()}>
            {(part, at) => {
              const last = () => at() === parts().length - 1;
              const prefix = () =>
                parts()
                  .slice(0, at() + 1)
                  .join("/");
              return (
                <>
                  <Show when={at() > 0}>
                    <i>/</i>
                  </Show>
                  <span classList={{ name: last() }} onClick={() => props.files.reveal(prefix(), !last())}>
                    {part}
                  </span>
                </>
              );
            }}
          </For>
        </span>
      </span>
      <Show when={renderable(props.path)}>
        <button
          class="modes"
          classList={{ code: !rendered() }}
          role="switch"
          aria-checked={rendered()}
          title={`${rendered() ? "Code" : "Rendered"} · ${hint(commands.rendered)}`}
          onClick={() => props.files.toggleRendered()}
        >
          <i class="knob" />
          <span classList={{ on: rendered() }}>{EYE}</span>
          <span classList={{ on: !rendered() }}>{CODE}</span>
        </button>
      </Show>
    </div>
  );
}

export function Files(props: { visible: boolean }) {
  let host!: HTMLDivElement;
  let editor: Editor | undefined;
  let showing: Shown;
  const files = createMemo(() => {
    const open = openView();
    return open === undefined ? undefined : filesOf(open);
  });
  const active = createMemo<Shown>(
    () => {
      const taskFiles = files();
      const path = taskFiles?.state.active ?? null;
      return taskFiles !== undefined && path !== null && taskFiles.state.loaded[path]
        ? { files: taskFiles, path }
        : undefined;
    },
    undefined,
    { equals: (a, b) => a?.files === b?.files && a?.path === b?.path },
  );

  const rendered = () => {
    const shown = active();
    return shown !== undefined && shown.files.state.rendered[shown.path] ? shown : undefined;
  };

  const save = (event: KeyboardEvent) => {
    const shown = active();
    if (shown === undefined || event.code !== "KeyS" || event.shiftKey || event.altKey) return;
    if (!(mac ? event.metaKey : event.ctrlKey)) return;
    event.preventDefault();
    shown.files.save(shown.path);
  };

  const show = async (next: Shown) => {
    if (showing !== undefined && editor !== undefined) {
      const previous = showing.files.buffer(showing.path);
      if (previous !== undefined) previous.view = editor.saveViewState();
    }
    showing = next;
    if (next === undefined) {
      editor?.setModel(null);
      return;
    }
    const { createEditor } = await loadMonaco();
    if (editor === undefined) {
      editor = createEditor(host);
    }
    if (showing !== next) return;
    const buffer = next.files.buffer(next.path);
    editor.setModel(buffer?.model ?? null);
    if (buffer?.view) editor.restoreViewState(buffer.view);
  };
  createEffect(on(active, (next) => show(next).catch(report)));

  return (
    <div class="files" tabIndex={-1}>
      <div class="tree">
        <Show when={files()}>{(taskFiles) => <Tree files={taskFiles()} dir="." depth={0} />}</Show>
      </div>
      <div class="editor">
        <Show when={files()} fallback={<div class="etabs" />}>
          {(taskFiles) => <EditorTabs files={taskFiles()} visible={props.visible} />}
        </Show>
        <Show when={files()}>
          {(taskFiles) => (
            <Show when={taskFiles().state.active}>{(path) => <FileHead files={taskFiles()} path={path()} />}</Show>
          )}
        </Show>
        <div class="editor-body" onKeyDown={save}>
          <div class="editor-host" ref={host} />
          <Show when={rendered()} keyed>
            {(shown) => <Rendered files={shown.files} path={shown.path} handBack={() => editor?.focus()} />}
          </Show>
          <Show when={files()?.state.active}>
            {(path) => (
              <Show when={files()?.state.unreadable[path()]}>
                {(problem) => <div class="empty over-editor">{problem()}</div>}
              </Show>
            )}
          </Show>
        </div>
      </div>
    </div>
  );
}
