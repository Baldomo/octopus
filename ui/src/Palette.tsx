import { createEffect, createMemo, createSignal, For, on, onCleanup, Show } from "solid-js";
import { backend } from "./backend";
import { autofocus } from "./Dialog";
import type { Match } from "./generated";
import { commands as keyCommands, hint } from "./keys";
import { model, openTask, openView, report, reveal, serverLabel, showModal } from "./state";

type Item = { label: string; detail: string; run: () => void };

const SEARCH_DELAY_MS = 150;
/** File search starts at this many characters; shorter patterns match nearly every line. */
const MIN_QUERY = 2;
const RESULTS_PER_GROUP = 50;

const commands: Item[] = Object.values(keyCommands)
  .filter((command) => command !== keyCommands.search)
  .map((command) => ({ label: command.label, detail: hint(command), run: command.run }));

export function Palette() {
  const [query, setQuery] = createSignal("");
  const [chosen, setChosen] = createSignal(0);
  const [matches, setMatches] = createSignal<Match[]>([]);
  const needle = () => query().trim().toLowerCase();

  createEffect(
    on(needle, (text) => {
      setChosen(0);
      setMatches([]);
      const open = openView();
      if (text.length < MIN_QUERY || open === undefined) return;
      const at = open.at;
      const timer = setTimeout(() => {
        backend.fileSearch(at, query().trim()).then((found) => {
          if (needle() === text) setMatches(found.slice(0, RESULTS_PER_GROUP));
        }, report);
      }, SEARCH_DELAY_MS);
      onCleanup(() => clearTimeout(timer));
    }),
  );

  const groups = createMemo(() => {
    const text = needle();
    const open = openView();
    const tasks: Item[] = Object.values(model.tasks)
      .filter((view) => view.task.title.toLowerCase().includes(text))
      .slice(0, RESULTS_PER_GROUP)
      .map((view) => ({
        label: view.task.title,
        detail: `${view.section} · ${serverLabel(view.at.server)}`,
        run: () => openTask(view.key),
      }));
    const files: Item[] =
      open === undefined
        ? []
        : matches().map((found) => ({
            label: `${found.path}:${found.line}`,
            detail: found.text.trim(),
            run: () => reveal(open.key, found.path),
          }));
    return [
      { name: "commands", items: commands.filter((command) => command.label.toLowerCase().includes(text)) },
      { name: "tasks", items: tasks },
      { name: "files", items: files },
    ].filter((group) => group.items.length > 0);
  });
  const items = createMemo(() => groups().flatMap((group) => group.items));
  const run = (item: Item | undefined) => {
    if (item === undefined) return;
    showModal(null);
    item.run();
  };

  return (
    <div class="pane dialog">
      <div class="search">
        <input
          ref={autofocus}
          placeholder="Search tasks, files, commands"
          spellcheck={false}
          value={query()}
          onInput={(event) => setQuery(event.currentTarget.value)}
          onKeyDown={(event) => {
            const count = items().length;
            if (event.key === "ArrowDown") setChosen((index) => Math.min(index + 1, count - 1));
            else if (event.key === "ArrowUp") setChosen((index) => Math.max(index - 1, 0));
            else if (event.key === "Enter") run(items()[chosen()]);
            else return;
            event.preventDefault();
          }}
        />
        <kbd>esc</kbd>
      </div>
      <div class="results">
        <For each={groups()}>
          {(group) => (
            <div class="group">
              <div class="gh">{group.name}</div>
              <For each={group.items}>
                {(item) => (
                  <div
                    class="task"
                    classList={{ active: items()[chosen()] === item }}
                    onPointerMove={() => setChosen(items().indexOf(item))}
                    onClick={() => run(item)}
                  >
                    <span class="tt">{item.label}</span>
                    <span class="srv name">{item.detail}</span>
                  </div>
                )}
              </For>
            </div>
          )}
        </For>
        <Show when={items().length === 0}>
          <div class="empty">nothing matches</div>
        </Show>
      </div>
    </div>
  );
}
