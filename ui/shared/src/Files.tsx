import { createEffect, createMemo, For, lazy, on, Show } from "solid-js";
import { commands, hint, mac } from "./commands";
import { FileTree, type Outside } from "./FileTree";
import { FolderTabs } from "./FolderTabs";
import type { LiftOnDrag } from "./lift";
import { MenuHost } from "./Menu";
import { Toast } from "./SidebarControls";
import { filesOf, loadMonaco, nameOf, renderable, tabHints, type TaskFiles } from "./taskFiles";
import { act, openView, report } from "./state";

type Editor = import("monaco-editor").editor.IStandaloneCodeEditor;
type Shown = { files: TaskFiles; path: string } | undefined;

/** A tab names its file, and the fewest trailing directories that tell it from another open tab of the same name. */
function EditorTabs(props: { files: TaskFiles; visible: boolean; drag: LiftOnDrag }) {
  const tabs = createMemo(() => {
    const { tabs, dirty } = props.files.state;
    const where = tabHints(tabs);
    return tabs.map((path) => ({
      key: path,
      name: nameOf(path),
      where: where[path],
      title: path,
      loading: false,
      dirty: dirty[path] === true,
    }));
  });
  return (
    <FolderTabs
      tabs={tabs()}
      selected={props.files.state.active}
      visible={props.visible}
      drag={props.drag}
      choose={(path) => act(props.files.open(path))}
      close={(path) => props.files.close(path)}
      move={async (path, to) => props.files.moveTab(path, to)}
    />
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

export function Files(props: { visible: boolean; drag: LiftOnDrag; outside?: Outside }) {
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
      <Show when={files()} keyed fallback={<div class="tree" />}>
        {(taskFiles) => <FileTree files={taskFiles} task={openView()!} outside={props.outside} />}
      </Show>
      <div class="editor">
        <Show when={files()}>
          {(taskFiles) => (
            <Show when={taskFiles().state.tabs.length > 0}>
              <EditorTabs files={taskFiles()} visible={props.visible} drag={props.drag} />
            </Show>
          )}
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
      <Show when={props.visible}>
        <MenuHost />
        <Toast />
      </Show>
    </div>
  );
}
