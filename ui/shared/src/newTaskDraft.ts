import { createSignal } from "solid-js";
import type { CheckoutKind, NewTask, TaskState, WorkspaceRef } from "./generated";
import { openView, serverView, workspaces } from "./state";

export const workspaceKey = (workspace: WorkspaceRef) => `${workspace.server}/${workspace.workspace}`;

/**
 * The fields of a new task, prefilled from the open task where it has one. Its workspace starts as `section`'s on its
 * first reachable server, or the open task's when `section` is null. It is complete once the workspace, title and
 * harness are set.
 */
export function newTaskDraft(section: string | null) {
  const current = openView();
  const initial = workspaces().find((workspace) =>
    section === null
      ? workspace.server === current?.at.server && workspace.name === current.section
      : workspace.name === section && serverView(workspace.server)?.reachable,
  );
  const [workspace, setWorkspace] = createSignal(initial === undefined ? "" : workspaceKey(initial));
  const [title, setTitle] = createSignal("");
  const [state, setState] = createSignal<TaskState>("pinned");
  const [checkout, setCheckout] = createSignal<CheckoutKind>("worktree");
  const [harness, setHarness] = createSignal(current?.task.harness ?? "");
  const chosen = () => workspaces().find((candidate) => workspaceKey(candidate) === workspace());
  const complete = () => chosen() !== undefined && title().trim() !== "" && harness().trim() !== "";
  const payload = (target: WorkspaceRef, parent: string | null, session: string): NewTask => ({
    server: target.server,
    workspace: target.workspace,
    title: title().trim(),
    parent,
    state: state(),
    checkout: checkout(),
    harness: harness().trim(),
    session,
  });
  return {
    current,
    workspace,
    setWorkspace,
    title,
    setTitle,
    state,
    setState,
    checkout,
    setCheckout,
    harness,
    setHarness,
    chosen,
    complete,
    payload,
  };
}
