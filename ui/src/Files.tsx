import { createEffect, createMemo, For, on, Show } from "solid-js";
import { filesOf, join, loadMonaco, nameOf, type TaskFiles } from "./taskFiles";
import { act, openView, report } from "./state";

type Editor = import("monaco-editor").editor.IStandaloneCodeEditor;
type Shown = { files: TaskFiles; path: string } | undefined;

/** DESIGN's tree indents each level, and spaces the modified mark, with two non-breaking spaces. */
const GAP = "  ";
const indent = (depth: number) => GAP.repeat(depth);

function Tree(props: { files: TaskFiles; dir: string; depth: number }) {
  return (
    <For each={props.files.state.listings[props.dir] ?? []}>
      {(entry) => {
        const path = join(props.dir, entry.name);
        if (entry.kind === "directory") {
          const expanded = () => props.files.state.expanded[path] === true;
          return (
            <>
              <div class="dir" onClick={() => props.files.toggle(path)}>
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

export function Files() {
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
    const { createEditor, monaco } = await loadMonaco();
    if (editor === undefined) {
      editor = createEditor(host);
      editor.addCommand(monaco.KeyMod.CtrlCmd | monaco.KeyCode.KeyS, () => {
        if (showing !== undefined) showing.files.save(showing.path);
      });
    }
    if (showing !== next) return;
    const buffer = next.files.buffer(next.path);
    editor.setModel(buffer?.model ?? null);
    if (buffer?.view) editor.restoreViewState(buffer.view);
  };
  createEffect(on(active, (next) => show(next).catch(report)));

  return (
    <div class="files">
      <div class="tree">
        <Show when={files()}>{(taskFiles) => <Tree files={taskFiles()} dir="." depth={0} />}</Show>
      </div>
      <div class="editor">
        <div class="etabs">
          <Show when={files()}>
            {(taskFiles) => (
              <For each={taskFiles().state.tabs}>
                {(path) => (
                  <span
                    classList={{ on: taskFiles().state.active === path }}
                    title={path}
                    onClick={() => act(taskFiles().open(path))}
                    onAuxClick={(event) => event.button === 1 && taskFiles().close(path)}
                  >
                    {nameOf(path)}
                  </span>
                )}
              </For>
            )}
          </Show>
        </div>
        <div class="editor-body">
          <div class="editor-host" ref={host} />
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
