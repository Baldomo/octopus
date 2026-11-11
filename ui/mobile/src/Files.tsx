import { createEffect, createMemo, For, Match, on, onCleanup, Show, Switch } from "solid-js";
import { keepInView } from "@octopus/shared/keepInView.ts";
import { Media } from "@octopus/shared/Media.tsx";
import { Rendered } from "@octopus/shared/Rendered.tsx";
import { act, scale, model, ui } from "@octopus/shared/state.ts";
import { filesOf, join, loadMonaco, nameOf, tabHints, type TaskFiles } from "@octopus/shared/taskFiles.ts";
import { token } from "@octopus/shared/tokens.ts";
import { animateList } from "@octopus/shared/flip.ts";
import { slidingThumb } from "./thumb";
import { Icon } from "./Icon";
import { onPainted } from "./motion";
import { back, go } from "./nav";
import { Circle, CrumbBar, Id, sectionColor, Slot, Trail } from "./parts";

type Editor = import("monaco-editor").editor.IStandaloneCodeEditor;

/** The phone's editor sets 13pt Meslo on 20pt rows, its line numbers ending 8 before the 52 rail. */
const FONT_PX = 13;
const ROW_PX = 20;

function Tree(props: { files: TaskFiles; taskKey: string; dir: string; depth: number }) {
  return (
    <For each={props.files.state.listings[props.dir] ?? []}>
      {(entry) => {
        const path = join(props.dir, entry.name);
        if (entry.kind === "directory") {
          const expanded = () => props.files.state.expanded[path] === true;
          return (
            <>
              <div
                class="tree dir"
                data-key={path}
                classList={{ closed: !expanded() }}
                style={{ "--d": props.depth }}
                role="button"
                onClick={() => props.files.toggle(path)}
              >
                <Slot>
                  <Icon name="car" class="car" />
                </Slot>
                <span class="name">{entry.name}</span>
              </div>
              <Show when={expanded()}>
                <Tree files={props.files} taskKey={props.taskKey} dir={path} depth={props.depth + 1} />
              </Show>
            </>
          );
        }
        const dirty = () => props.files.state.dirty[path] === true;
        return (
          <div
            class="tree file"
            data-key={path}
            classList={{ unsaved: dirty() }}
            style={{ "--d": props.depth }}
            role="button"
            onClick={() => {
              act(props.files.open(path));
              go({ at: "editor", key: props.taskKey });
            }}
          >
            <Slot>
              <Icon name="file" />
            </Slot>
            <span class="name">{entry.name}</span>
            <Show when={dirty()}>
              <span class="count">
                <i class="dot" />
              </span>
            </Show>
          </div>
        );
      }}
    </For>
  );
}

export function Files(props: { taskKey: string }) {
  return (
    <Show when={model.tasks[props.taskKey]}>
      {(view) => {
        const files = filesOf(view());
        return (
          <>
            <CrumbBar
              up="Task"
              color={sectionColor(view().section)}
              crumb={<span class="name">{view().task.title}</span>}
              title="Files"
              end={<Circle icon="search" label="Search" tap={() => go({ at: "search" })} />}
            />
            <div class="cards" ref={animateList(".tree")}>
              <Tree files={files} taskKey={props.taskKey} dir="." depth={0} />
            </div>
          </>
        );
      }}
    </Show>
  );
}

const sized = () => ({
  fontSize: FONT_PX * scale(ui.prefs),
  lineHeight: ROW_PX * scale(ui.prefs),
  lineDecorationsWidth: 6 * scale(ui.prefs),
  padding: { top: 8 * scale(ui.prefs) },
});

function Code(props: { files: TaskFiles; path: string }) {
  let host!: HTMLDivElement;
  let editor: Editor | undefined;
  let gone = false;
  onPainted(() =>
    loadMonaco().then(({ monaco }) => {
      if (gone) return;
      editor = monaco.editor.create(host, {
        ...sized(),
        model: null,
        fontFamily: token("--mono-terminal"),
        lineNumbersMinChars: 6,
        glyphMargin: false,
        folding: false,
        minimap: { enabled: false },
        automaticLayout: true,
        scrollBeyondLastLine: false,
        renderLineHighlight: "none",
        bracketPairColorization: { enabled: false },
        guides: { indentation: false },
        matchBrackets: "never",
        occurrencesHighlight: "off",
        overviewRulerLanes: 0,
        hideCursorInOverviewRuler: true,
        scrollbar: { vertical: "hidden", horizontal: "hidden" },
      });
      editor.setModel(props.files.buffer(props.path)?.model ?? null);
    }),
  );
  createEffect(
    on(
      () => props.files.state.views[props.path],
      () => editor?.setModel(props.files.buffer(props.path)?.model ?? null),
    ),
  );
  createEffect(() => {
    const options = sized();
    editor?.updateOptions(options);
  });
  onCleanup(() => {
    gone = true;
    editor?.dispose();
  });
  return (
    <div class="codepane fill">
      <div class="zoom-reset">
        <div ref={host} class="code-host" />
      </div>
    </div>
  );
}

export function EditorScreen(props: { taskKey: string }) {
  return (
    <Show when={model.tasks[props.taskKey]}>
      {(view) => {
        const files = filesOf(view());
        const path = () => files.state.active;
        const hints = createMemo(() => tabHints(files.state.tabs));
        return (
          <Show when={path()} keyed>
            {(shown) => (
              <>
                <div class="bar">
                  <Circle icon="back" label="Files" tap={back} />
                  <div class="ttl">
                    <span class="crumb">
                      <Trail parts={shown.split("/").slice(0, -1)} />
                    </span>
                    <b class="name">{nameOf(shown)}</b>
                  </div>
                  <Show when={files.switchable(shown)}>
                    <div class="capsule">
                      <Circle
                        icon="eye"
                        label="Rendered"
                        on={files.state.rendered[shown] === true}
                        tap={() => files.state.rendered[shown] || files.toggleRendered()}
                      />
                      <Circle
                        icon="code"
                        label="Code"
                        on={files.state.rendered[shown] !== true}
                        tap={() => files.state.rendered[shown] && files.toggleRendered()}
                      />
                    </div>
                  </Show>
                  <Show when={files.state.dirty[shown]}>
                    <button class="btn solid" onClick={() => files.save(shown)}>
                      Save
                    </button>
                  </Show>
                </div>
                <div class="tabs scroll" ref={slidingThumb}>
                  <For each={files.state.tabs}>
                    {(tab) => (
                      <span
                        ref={keepInView(() => tab === shown)}
                        class="tab"
                        classList={{ on: tab === shown }}
                        role="button"
                        onClick={() => act(files.open(tab))}
                      >
                        {nameOf(tab)}
                        <Show when={files.state.dirty[tab]}>
                          <i class="udot" />
                        </Show>
                        <Show when={hints()[tab]}>{(hint) => <Id>{hint()}</Id>}</Show>
                      </span>
                    )}
                  </For>
                </div>
                <Show when={files.state.unreadable[shown]}>{(problem) => <div class="empty">{problem()}</div>}</Show>
                <Switch fallback={<Code files={files} path={shown} />}>
                  <Match when={files.media(shown)} keyed>
                    {(view) => (
                      <div class="fill rendered-host">
                        <Media files={files} path={shown} view={view} />
                      </div>
                    )}
                  </Match>
                  <Match when={files.state.rendered[shown] && files.state.views[shown]?.as === "text"}>
                    <div class="fill rendered-host">
                      <Rendered files={files} path={shown} handBack={() => {}} />
                    </div>
                  </Match>
                </Switch>
              </>
            )}
          </Show>
        );
      }}
    </Show>
  );
}
