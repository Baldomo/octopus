import { createSignal, For, onCleanup, onMount, Show, type JSX } from "solid-js";
import { Actions, autofocus, Field, Segments, TextField } from "./Dialog";
import type { FolderPicker } from "./folderPath";
import { MODES, TASK_STATES } from "./generated";
import { dismissible } from "./Menu";
import { floatPanel, shiftIn } from "./motion";
import { modeLabel, newTaskDraft, workspaceKey } from "./newTaskDraft";
import { newTaskSection, openTask, serverLabel, setNewTaskSection, showModal, workspaces } from "./state";

type Draft = ReturnType<typeof newTaskDraft>;

/** A folder as a field shows it: what leads it dimmed, the path after, and its repo's tag. */
function FolderPath(props: { lead: string; tag: string | null; children: JSX.Element }) {
  return (
    <>
      <Show when={props.lead}>
        <span class="root">
          <bdi>{props.lead}</bdi>
        </span>
      </Show>
      {props.children}
      <Show when={props.tag}>{(tag) => <span class="tag">{tag()}</span>}</Show>
    </>
  );
}

/**
 * The folder picker floating where the folder field was: the path typed under the workspace's root above a list of
 * the folders completing it. Arrows point at a folder, Tab or a click goes into it, Enter or Choose takes the path, and
 * Escape or a press outside cancels.
 */
function FolderPick(props: { picker: FolderPicker; gone: () => void }) {
  const picker = props.picker;
  const [pointed, setPointed] = createSignal(-1);
  let panel!: HTMLDivElement;
  let list!: HTMLDivElement;
  let leaving = false;
  const rows = () => picker.completions();
  const leave = () => {
    leaving = true;
    void floatPanel(panel, true).then(props.gone);
  };
  const choose = () => !leaving && picker.choose() && leave();
  const cancel = () => !leaving && leave();
  const type = (path: string) => {
    picker.setPath(path);
    setPointed(path.endsWith("/") || path === "" ? -1 : 0);
  };
  const point = (index: number) => {
    setPointed(index);
    const row = rows()[index];
    if (row !== undefined) picker.peek(row.into);
    list.querySelectorAll(".mi")[index]?.scrollIntoView({ block: "nearest" });
  };
  const keys = (event: KeyboardEvent) => {
    const row = rows()[pointed()];
    if (event.key === "ArrowDown") point(Math.min(pointed() + 1, rows().length - 1));
    else if (event.key === "ArrowUp") point(Math.max(pointed() - 1, -1));
    else if (event.key === "Tab" && !event.shiftKey) type(row?.into ?? picker.tab());
    else if (event.key === "Enter") {
      if (row !== undefined) picker.setPath(row.pick);
      choose();
    } else return;
    event.preventDefault();
  };
  picker.onTurn((toward) => shiftIn(list, toward));
  onCleanup(picker.cancel);
  onMount(() => {
    void floatPanel(panel);
    dismissible(panel, cancel);
  });
  return (
    <div
      ref={panel}
      class="menu folder-pick"
      role="dialog"
      onMouseDown={(event) => event.target instanceof HTMLInputElement || event.preventDefault()}
    >
      <label class="search path-field">
        <FolderPath lead={picker.prefix()} tag={picker.tag()}>
          <input
            ref={autofocus}
            spellcheck={false}
            value={picker.path()}
            style={{ width: `${picker.path().length + 1}ch` }}
            onInput={(event) => type(event.currentTarget.value)}
            onKeyDown={keys}
          />
        </FolderPath>
      </label>
      <div ref={list} class="folders">
        <For each={rows()}>
          {(row, index) => (
            <button
              type="button"
              class="mi"
              classList={{ active: index() === pointed() }}
              onMouseEnter={() => picker.peek(row.into)}
              onClick={() => type(row.into)}
            >
              <span class="name">{row.name}/</span>
            </button>
          )}
        </For>
        <Show when={picker.listing().note}>
          {(note) => (
            <div class="mlabel" classList={{ err: note().refused }}>
              {note().text}
            </div>
          )}
        </Show>
      </div>
      <div class="sep" />
      <div class="pick-foot">
        <span>⇥ goes in · ⏎ chooses · esc cancels</span>
        <button type="button" class="btn live" disabled={picker.folder() === undefined} onClick={choose}>
          Choose
        </button>
      </div>
    </div>
  );
}

/** The folder field, which raises the folder picker over itself and takes the keyboard back once it lowers. */
function FolderField(props: { draft: Draft }) {
  const [picker, setPicker] = createSignal<FolderPicker>();
  let field!: HTMLButtonElement;
  return (
    <div class="anchor">
      <button
        ref={field}
        type="button"
        class="search path-field"
        aria-haspopup="dialog"
        onClick={() => setPicker(props.draft.pickFolder())}
      >
        <FolderPath lead={props.draft.lead()} tag={props.draft.tag()}>
          <span class="typed">{props.draft.path()}</span>
        </FolderPath>
      </button>
      <Show when={picker()} keyed>
        {(open) => (
          <FolderPick
            picker={open}
            gone={() => {
              setPicker(undefined);
              field.focus();
            }}
          />
        )}
      </Show>
    </div>
  );
}

export function NewTask() {
  const draft = newTaskDraft(newTaskSection());
  setNewTaskSection(null);

  const create = (event: SubmitEvent) => {
    event.preventDefault();
    void draft.create().then((key) => {
      if (key === undefined || key === null) return;
      showModal(null);
      openTask(key);
    });
  };

  return (
    <form class="pane dialog new-task" onSubmit={create}>
      <Show
        when={workspaces().length > 0}
        fallback={<div class="empty">No server lists a workspace yet; add one with + workspace in the sidebar.</div>}
      >
        <TextField label="title" focus value={draft.title()} change={draft.setTitle} />
        <Field label="workspace">
          <div class="search">
            <select value={draft.workspace()} onChange={(event) => draft.setWorkspace(event.currentTarget.value)}>
              <Show when={draft.chosen() === undefined}>
                <option value="">choose a workspace</option>
              </Show>
              <For each={workspaces()}>
                {(option) => (
                  <option value={workspaceKey(option)} disabled={!draft.reachable(option)}>
                    {option.name} · {serverLabel(option.server)}
                  </option>
                )}
              </For>
            </select>
          </div>
        </Field>
        <Field label="folder">
          <FolderField draft={draft} />
        </Field>
        <Field label="mode">
          <Segments options={MODES} value={draft.mode()} choose={draft.setMode} label={modeLabel} off={draft.modeOff} />
        </Field>
        <Field label="parent">
          <div class="search">
            <select
              value={draft.parent() ?? ""}
              onChange={(event) => draft.setParent(event.currentTarget.value || null)}
            >
              <option value="">none</option>
              <For each={draft.parents()}>{(view) => <option value={view.task.id}>{view.task.title}</option>}</For>
            </select>
          </div>
        </Field>
        <Field label="state">
          <Segments options={TASK_STATES} value={draft.state()} choose={draft.setState} />
        </Field>
        <TextField label="harness" value={draft.harness()} change={draft.setHarness} />
        <TextField
          label="session"
          value={draft.session()}
          placeholder="fresh session, or an id to resume"
          change={draft.setSession}
        />
        <div class="foot">
          <Show when={draft.refusal()}>{(message) => <div class="err">{message()}</div>}</Show>
          <Actions submit="Create" complete={draft.complete() && !draft.creating()} />
        </div>
      </Show>
    </form>
  );
}
