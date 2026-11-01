import { createSignal, For, Show } from "solid-js";
import { backend } from "@octopus/shared/backend.ts";
import { CHECKOUTS, TASK_STATES, type Checkout, type TaskState, type WorkspaceRef } from "@octopus/shared/generated.ts";
import { model, openView, report, serverLabel, serverView, workspaces } from "@octopus/shared/state.ts";
import { keepInView } from "@octopus/shared/keepInView.ts";
import { feel } from "./feel";
import { Icon } from "./Icon";
import { openTask, showInSheet } from "./nav";
import { Field, Flab, Gap, Para, Secondary, sectionColor, Seg } from "./parts";
import { Sheet, SheetHead } from "./Sheet";

const workspaceKey = (workspace: WorkspaceRef) => `${workspace.server}/${workspace.workspace}`;

/**
 * Every field of `task.create` on one full sheet, with `section`'s workspace on its first reachable server chosen.
 * Create stays inert until the title, workspace and harness are set.
 */
export function NewTask(props: { section: string | null }) {
  const current = openView();
  const initial = workspaces().find((workspace) =>
    props.section === null
      ? workspace.server === current?.at.server && workspace.name === current.section
      : workspace.name === props.section && serverView(workspace.server)?.reachable,
  );
  const [workspace, setWorkspace] = createSignal(initial === undefined ? "" : workspaceKey(initial));
  const [title, setTitle] = createSignal("");
  const [parent, setParent] = createSignal("");
  const [state, setState] = createSignal<TaskState>("pinned");
  const [checkout, setCheckout] = createSignal<Checkout>("worktree");
  const [harness, setHarness] = createSignal(current?.task.harness ?? "");
  const [session, setSession] = createSignal("");
  const chosen = () => workspaces().find((candidate) => workspaceKey(candidate) === workspace());
  const parents = () =>
    Object.values(model.tasks).filter(
      (view) =>
        view.live &&
        view.at.server === chosen()?.server &&
        view.task.workspace === chosen()?.workspace &&
        view.task.archived_at === null,
    );
  const complete = () => chosen() !== undefined && title().trim() !== "" && harness().trim() !== "";
  const [creating, setCreating] = createSignal(false);
  const create = () => {
    const target = chosen();
    if (target === undefined || !complete() || creating()) return;
    setCreating(true);
    backend
      .taskCreate({
        server: target.server,
        workspace: target.workspace,
        title: title().trim(),
        parent: parents().some((view) => view.task.id === parent()) ? parent() : null,
        state: state(),
        checkout: checkout(),
        harness: harness().trim(),
        session: session().trim() === "" ? null : session().trim(),
      })
      .then((key) => {
        feel("success");
        openTask(key, { server: target.server, task: key.slice(key.lastIndexOf("/") + 1) });
      }, report)
      .finally(() => setCreating(false));
  };
  const parentTitle = () => parents().find((view) => view.task.id === parent())?.task.title ?? "none";
  return (
    <Show
      when={workspaces().length > 0}
      fallback={
        <Sheet>
          <SheetHead title="New task" />
          <Para>
            No server lists a workspace yet. Add one with <b>+ workspace</b> at the end of Tasks.
          </Para>
          <Gap n={3} />
          <Secondary tap={() => showInSheet({ is: "addws" }, "deeper")}>Add a workspace</Secondary>
          <Gap n={2} />
        </Sheet>
      }
    >
      <Sheet full>
        <SheetHead
          title="New task"
          end={
            <button class="btn solid" classList={{ inert: !complete() || creating() }} onClick={create}>
              Create
            </button>
          }
        />
        <Flab>title</Flab>
        <Field value={title()} input={setTitle} focus />
        <Flab>workspace</Flab>
        <div class="pick">
          <For each={workspaces()}>
            {(option) => (
              <span
                ref={keepInView(() => workspaceKey(option) === workspace())}
                class="chip"
                role="button"
                classList={{ on: workspaceKey(option) === workspace(), off: !serverView(option.server)?.reachable }}
                onClick={() => {
                  if (workspaceKey(option) !== workspace()) feel("select");
                  setWorkspace(workspaceKey(option));
                }}
              >
                <i class="dot srv" style={{ background: sectionColor(option.name) }} />
                {option.name} · {serverLabel(option.server)}
              </span>
            )}
          </For>
        </div>
        <Flab>parent</Flab>
        <label class="field">
          <span class="name">{parentTitle()}</span>
          <span class="chev">
            <Icon name="chev" />
          </span>
          <select class="native" value={parent()} onChange={(event) => setParent(event.currentTarget.value)}>
            <option value="">none</option>
            <For each={parents()}>{(view) => <option value={view.task.id}>{view.task.title}</option>}</For>
          </select>
        </label>
        <Flab>state</Flab>
        <Seg wide options={TASK_STATES.map((each) => [each, each] as const)} value={state()} choose={setState} />
        <Flab>checkout</Flab>
        <Seg
          wide
          options={[...CHECKOUTS].reverse().map((each) => [each, each] as const)}
          value={checkout()}
          choose={setCheckout}
        />
        <Flab>harness</Flab>
        <Field value={harness()} input={setHarness} mono />
        <Flab>session</Flab>
        <Field value={session()} input={setSession} placeholder="fresh session, or an id to resume" />
        <Gap n={2} />
      </Sheet>
    </Show>
  );
}
