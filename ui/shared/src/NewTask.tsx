import { createSignal, For, Show } from "solid-js";
import { backend } from "./backend";
import { Actions, Field, Segments, TextField } from "./Dialog";
import { CHECKOUTS, FRESH_SESSION, TASK_STATES } from "./generated";
import { newTaskDraft, workspaceKey } from "./newTaskDraft";
import { newTaskSection, openTask, report, serverLabel, setNewTaskSection, showModal, workspaces } from "./state";

const PARENTS = ["none", "open task"] as const;

export function NewTask() {
  const draft = newTaskDraft(newTaskSection());
  setNewTaskSection(null);
  const { current, workspace, setWorkspace, title, setTitle, state, setState, checkout, setCheckout } = draft;
  const { harness, setHarness, chosen, complete } = draft;
  const [parent, setParent] = createSignal<(typeof PARENTS)[number]>("none");
  const underOpen = () => current !== undefined && chosen()?.server === current.at.server;

  const create = (event: SubmitEvent) => {
    event.preventDefault();
    const target = chosen();
    if (target === undefined || !complete()) return;
    backend
      .taskCreate(
        draft.payload(target, underOpen() && parent() === "open task" ? current!.task.id : null, FRESH_SESSION),
      )
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
