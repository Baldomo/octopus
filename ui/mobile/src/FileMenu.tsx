import { createSignal, Show } from "solid-js";
import {
  absolute,
  copyPaths,
  create,
  deletion,
  duplicate,
  folderOf,
  paste,
  remove,
  rename,
  type Say,
} from "@octopus/shared/entries.ts";
import type { EntryKind, TaskView } from "@octopus/shared/generated.ts";
import { openShell } from "@octopus/shared/panels.ts";
import { model, report } from "@octopus/shared/state.ts";
import { filesOf, nameOf } from "@octopus/shared/taskFiles.ts";
import { Icon } from "./Icon";
import { back, go, up } from "./nav";
import { Confirm, Gap, Mi, Sep, Why } from "./parts";
import { NameItem } from "./NameField";
import { Sheet, SheetHead } from "./Sheet";
import { notify } from "./Toast";

const SAY: Say = { done: notify, problem: (text) => void report(text) };

function Items(props: { view: TaskView; path: string; kind: EntryKind }) {
  const files = filesOf(props.view);
  const root = props.path === ".";
  const dir = folderOf(props.path, props.kind);
  const paths = [props.path];
  const asked = () => deletion(props.view, paths);
  const [deleting, setDeleting] = createSignal(false);
  const closing = (action: () => unknown) => () => {
    back();
    action();
  };
  const make = (kind: EntryKind) => async (name: string) => {
    await create(files, dir, name, kind);
    if (kind === "file") go({ at: "editor", key: props.view.key });
    else back();
  };
  return (
    <Show
      when={!deleting()}
      fallback={
        <Confirm
          text={asked().question}
          act={asked().button}
          run={closing(() => remove(files, paths, asked().trash, SAY))}
          cancel={() => setDeleting(false)}
        />
      }
    >
      <NameItem icon="file" label="New file…" value="" apply={make("file")} />
      <NameItem icon="folder" label="New folder…" value="" apply={make("directory")} />
      <Mi
        off={!props.view.live}
        end={<Show when={!props.view.live}>{<Why>task ended</Why>}</Show>}
        tap={() => openShell(props.view.at, props.view.key, dir).then(up, report)}
      >
        Open in terminal
      </Mi>
      <Sep />
      <Show when={!root}>
        <Mi tap={closing(() => files.setClip({ paths, cut: true }))}>Cut</Mi>
        <Mi slot={<Icon name="copy" />} tap={closing(() => files.setClip({ paths, cut: false }))}>
          Copy
        </Mi>
      </Show>
      <Mi
        slot={<Icon name="paste" />}
        off={files.state.clip === null}
        end={<Show when={files.state.clip === null}>{<Why>nothing copied</Why>}</Show>}
        tap={closing(() => paste(files, dir, SAY))}
      >
        Paste
      </Mi>
      <Show when={!root}>
        <Mi tap={closing(() => duplicate(files, paths, SAY))}>Duplicate</Mi>
      </Show>
      <Sep />
      <Mi slot={<Icon name="copy" />} tap={closing(() => copyPaths(props.view, paths, true, SAY))}>
        Copy path
      </Mi>
      <Show when={!root}>
        <Mi slot={<Icon name="copy" />} tap={closing(() => copyPaths(props.view, paths, false, SAY))}>
          Copy relative path
        </Mi>
        <Sep />
        <NameItem
          icon="pen"
          label="Rename…"
          value={nameOf(props.path)}
          apply={(name) => rename(files, props.path, name).then(back)}
        />
        <Mi slot={<Icon name="x" />} danger tap={() => setDeleting(true)}>
          Delete…
        </Mi>
      </Show>
      <Show when={root}>
        <Sep />
        <Mi tap={closing(() => files.collapseAll())}>Collapse all folders</Mi>
      </Show>
    </Show>
  );
}

/**
 * A tree entry's sheet, or the task directory's at `.`: it makes, holds, pastes, copies the path of, renames and
 * deletes entries on the task's server, and opens a shell in the entry's folder.
 */
export function FileMenu(props: { taskKey: string; path: string; kind: EntryKind }) {
  return (
    <Sheet>
      <Show when={model.tasks[props.taskKey]}>
        {(view) => (
          <>
            <SheetHead
              title={props.path === "." ? "Task directory" : nameOf(props.path)}
              facts={[absolute(view(), props.path)]}
            />
            <Items view={view()} path={props.path} kind={props.kind} />
            <Gap n={2} />
          </>
        )}
      </Show>
    </Sheet>
  );
}
