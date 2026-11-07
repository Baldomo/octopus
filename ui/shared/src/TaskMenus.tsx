import { For, Show } from "solid-js";
import { backend } from "./backend";
import { LIVE_STATES, TASK_STATES, type Row, type Section, type TaskState } from "./generated";
import { commands, hint } from "./commands";
import { closeMenu, closing, Head, Item, Sep } from "./Menu";
import { Matrix, startRename } from "./SidebarControls";
import { act, model, report, serverLabel, serverView, setNewTaskSection, showBoard, showModal } from "./state";
import { swatchColor } from "./tokens";
import { copy, toast } from "./Toast";

export function newTaskIn(section: string) {
  setNewTaskSection(section);
  showModal("new_task");
}

const liveOn = (section: Section, server: string) =>
  section.groups.reduce((count, group) => count + group.rows.filter((row) => row.at.server === server).length, 0);

export const isLive = (state: TaskState) => LIVE_STATES.some((live) => live === state);

/** Finds its section by name on every model, so it follows changes made while it is open. */
export function SectionMenu(props: { name: string }) {
  const section = () => model.lists.sections.find((candidate) => candidate.name === props.name);
  const workspaces = () => model.lists.workspaces.filter((workspace) => workspace.name === props.name);
  return (
    <Show when={section()}>
      {(section) => (
        <>
          <Head
            title={props.name}
            color={swatchColor(section().swatch)}
            lines={workspaces().map((workspace) => `${serverLabel(workspace.server)}  ${workspace.root}`)}
          />
          <Item kbd={hint(commands.newTask)} run={closing(() => newTaskIn(props.name))}>
            New task here
          </Item>
          <Item run={closing(() => showBoard(props.name))}>Open board</Item>
          <Item run={closing(() => startRename("section", props.name))}>Rename…</Item>
          <div class="mlabel">Color</div>
          <Matrix
            current={section().swatch}
            auto={section().auto}
            pick={(swatch) => backend.sectionColor(props.name, swatch).catch(report)}
          />
          <Sep />
          <For each={workspaces()}>
            {(workspace) => {
              const live = () => liveOn(section(), workspace.server);
              const why = () =>
                !serverView(workspace.server)?.reachable ? "unreachable" : live() > 0 ? `${live()} live` : undefined;
              const remove = () =>
                backend.workspaceRemove(workspace.server, workspace.workspace).then(
                  () =>
                    toast(
                      <>
                        Removed <b>{workspace.name}</b> from {serverLabel(workspace.server)} · branches kept
                      </>,
                    ),
                  report,
                );
              return (
                <Item danger why={why()} run={closing(remove)}>
                  <span class="name">Remove from {serverLabel(workspace.server)}</span>
                </Item>
              );
            }}
          </For>
        </>
      )}
    </Show>
  );
}

export function TaskMenu(props: { row: Row }) {
  const move = (state: TaskState) =>
    closing(() => {
      if (state === props.row.state) return;
      act(backend.taskMove(props.row.at, state));
      toast(
        <>
          <b>{props.row.title}</b> → {state}
        </>,
      );
    });
  return (
    <>
      <Head title={props.row.title} lines={[`${props.row.at.task} · ${serverLabel(props.row.at.server)}`]} />
      <Item kbd="F2" run={closing(() => startRename("task", props.row.key))}>
        Rename…
      </Item>
      <Sep />
      <For each={TASK_STATES}>
        {(state) => (
          <Item check={state === props.row.state} run={move(state)}>
            {state}
          </Item>
        )}
      </For>
      <Sep />
      <Item run={() => copy(props.row.at.task, <b>{props.row.at.task}</b>).then(closeMenu, report)}>Copy task id</Item>
      <Item
        why={isLive(props.row.state) ? "end it first" : undefined}
        run={closing(() => act(backend.taskArchive(props.row.at)))}
      >
        Archive
      </Item>
    </>
  );
}
