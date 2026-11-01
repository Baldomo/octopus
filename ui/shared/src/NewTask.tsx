import { createSignal, For, Show } from "solid-js";
import { backend } from "./backend";
import { Actions, Field, Segments, TextField } from "./Dialog";
import { CHECKOUTS, TASK_STATES, type Checkout, type TaskState, type WorkspaceRef } from "./generated";
import {
  newTaskSection,
  openTask,
  openView,
  report,
  serverLabel,
  serverView,
  setNewTaskSection,
  showModal,
  workspaces,
} from "./state";

const PARENTS = ["none", "open task"] as const;
const workspaceKey = (workspace: WorkspaceRef) => `${workspace.server}/${workspace.workspace}`;

/** The new-task form: every op field is on screen, prefilled from the open task where it has one. */
export function NewTask() {
  const current = openView();
  const section = newTaskSection();
  setNewTaskSection(null);
  const initial =
    section === null
      ? workspaces().find((workspace) => workspace.server === current?.at.server && workspace.name === current.section)
      : workspaces().find((workspace) => workspace.name === section && serverView(workspace.server)?.reachable);
  const [workspace, setWorkspace] = createSignal(initial === undefined ? "" : workspaceKey(initial));
  const [title, setTitle] = createSignal("");
  const [parent, setParent] = createSignal<(typeof PARENTS)[number]>("none");
  const [state, setState] = createSignal<TaskState>("pinned");
  const [checkout, setCheckout] = createSignal<Checkout>("worktree");
  const [harness, setHarness] = createSignal(current?.task.harness ?? "");
  const chosen = () => workspaces().find((candidate) => workspaceKey(candidate) === workspace());
  const underOpen = () => current !== undefined && chosen()?.server === current.at.server;
  const complete = () => chosen() !== undefined && title().trim() !== "" && harness().trim() !== "";

  const create = (event: SubmitEvent) => {
    event.preventDefault();
    const target = chosen();
    if (target === undefined || !complete()) return;
    backend
      .taskCreate({
        server: target.server,
        workspace: target.workspace,
        title: title().trim(),
        parent: underOpen() && parent() === "open task" ? current!.task.id : null,
        state: state(),
        checkout: checkout(),
        harness: harness().trim(),
        session: null,
      })
      .then((key) => {
        showModal(null);
        openTask(key);
      }, report);
  };

  return (
    <form class="pane dialog" onSubmit={create}>
      <Show
        when={workspaces().length > 0}
        fallback={<div class="empty">No server lists a workspace yet; add one with + workspace in the sidebar.</div>}
      >
        <TextField label="title" focus value={title()} change={setTitle} />
        <Field label="workspace">
          <div class="search">
            <select value={workspace()} onChange={(event) => setWorkspace(event.currentTarget.value)}>
              <Show when={chosen() === undefined}>
                <option value="">choose a workspace</option>
              </Show>
              <For each={workspaces()}>
                {(option) => (
                  <option value={workspaceKey(option)}>
                    {option.name} · {serverLabel(option.server)}
                  </option>
                )}
              </For>
            </select>
          </div>
        </Field>
        <Show when={underOpen()}>
          <Field label="parent">
            <Segments options={PARENTS} value={parent()} choose={setParent} />
          </Field>
        </Show>
        <Field label="state">
          <Segments options={TASK_STATES} value={state()} choose={setState} />
        </Field>
        <Field label="checkout">
          <Segments options={CHECKOUTS} value={checkout()} choose={setCheckout} />
        </Field>
        <TextField label="harness" value={harness()} change={setHarness} />
        <Actions submit="Create" complete={complete()} />
      </Show>
    </form>
  );
}
