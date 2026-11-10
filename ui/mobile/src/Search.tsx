import { createSignal, For, type JSX } from "solid-js";
import { fileMatches } from "@octopus/shared/fileMatches.ts";
import { APPEARANCES } from "@octopus/shared/ClientMenu.tsx";
import { act, model, openView, setAppearance } from "@octopus/shared/state.ts";
import { filesOf, nameOf } from "@octopus/shared/taskFiles.ts";
import { Icon } from "./Icon";
import { onPainted } from "./motion";
import { back, go, openSheet, openTask } from "./nav";
import { Gap, Gl, Glyph, Id, Mi, Para } from "./parts";

const PER_GROUP = 8;
/** Enough matched lines that a file with many of them leaves room for the others. */
const MATCHED_LINES = 1000;

type Result = { slot: JSX.Element; label: string; where?: string; run: () => void };

export function Search() {
  const [query, setQuery] = createSignal("");
  const needle = () => query().trim().toLowerCase();
  const matches = fileMatches(() => openView()?.at, query, MATCHED_LINES);
  const paths = () => [...new Set(matches().map((match) => match.path))].slice(0, PER_GROUP);
  let field!: HTMLInputElement;
  onPainted(() => field.focus({ preventScroll: true }));

  const tasks = (): Result[] =>
    needle() === ""
      ? []
      : Object.values(model.tasks)
          .filter((view) => view.task.archived_at === null && view.task.title.toLowerCase().includes(needle()))
          .slice(0, PER_GROUP)
          .map((view) => ({
            slot: <Glyph status={view.glyph} />,
            label: view.task.title,
            where: view.section,
            run: () => openTask(view.key),
          }));
  const files = (): Result[] => {
    const open = openView();
    if (open === undefined) return [];
    return paths().map((path) => ({
      slot: <Icon name="file" />,
      label: nameOf(path),
      where: open.task.title,
      run: () => {
        act(filesOf(open).open(path));
        go({ at: "editor", key: open.key });
      },
    }));
  };
  const commands = (): Result[] =>
    [
      { slot: <Icon name="plus" />, label: "New task", run: () => openSheet({ is: "newtask", section: null }) },
      { slot: <Icon name="qr" />, label: "Pair a server…", run: () => go({ at: "scan", reading: "invite" }) },
      ...APPEARANCES.map(([appearance, name]) => ({
        slot: <Icon name="sun" />,
        label: `Appearance: ${name}`,
        run: () => {
          setAppearance(appearance);
          back();
        },
      })),
    ].filter((command) => command.label.toLowerCase().includes(needle()));

  const groups = (): [string, Result[]][] =>
    (
      [
        ["tasks", tasks()],
        ["files", files()],
        ["commands", commands()],
      ] as [string, Result[]][]
    ).filter(([, results]) => results.length > 0);

  return (
    <>
      <div class="bar">
        <label class="url">
          <Icon name="search" />
          <input
            value={query()}
            placeholder="Tasks, files and commands"
            spellcheck={false}
            autocapitalize="off"
            autocorrect="off"
            enterkeyhint="search"
            ref={field}
            onInput={(event) => setQuery(event.currentTarget.value)}
          />
        </label>
        <span class="cancel" role="button" onClick={back}>
          Cancel
        </span>
      </div>
      <div class="cards">
        <For
          each={groups()}
          fallback={
            <>
              <Gap n={2} />
              <Para>
                No task, file or command matches <b>{query().trim()}</b>.
              </Para>
            </>
          }
        >
          {([name, results]) => (
            <>
              <Gl>{name}</Gl>
              <For each={results}>
                {(result) => (
                  <Mi slot={result.slot} end={result.where ? <Id>{result.where}</Id> : undefined} tap={result.run}>
                    {result.label}
                  </Mi>
                )}
              </For>
            </>
          )}
        </For>
      </div>
    </>
  );
}
