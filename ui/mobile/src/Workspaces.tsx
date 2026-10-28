import { createResource, createSignal, For, Show } from "solid-js";
import { backend } from "@octopus/shared/backend.ts";
import { HUES, TONES, type Section, type Swatch } from "@octopus/shared/generated.ts";
import { act, model, report, serverLabel, serverView, servers } from "@octopus/shared/state.ts";
import { swatchColor } from "@octopus/shared/tokens.ts";
import { Icon } from "./Icon";
import { back, go, screen, showInSheet } from "./nav";
import { Chev, Field, Flab, Gap, Mi, Note, Path, Sep, ServerDot, Why } from "./parts";
import { Rename } from "./Rename";
import { Sheet, SheetHead } from "./Sheet";

const same = (a: Swatch | null | undefined, b: Swatch | null | undefined) => a?.hue === b?.hue && a?.tone === b?.tone;

/** The 12 hues across and 4 tones down; the current color ringed and the auto color dotted, which picks auto. */
function Matrix(props: { on: Swatch | null; auto: Swatch | undefined; pick: (swatch: Swatch | null) => void }) {
  return (
    <div class="matrix">
      <For each={TONES}>
        {(tone) => (
          <For each={HUES}>
            {(hue) => {
              const swatch = { hue, tone };
              return (
                <i
                  role="button"
                  aria-label={`${hue} ${tone}`}
                  classList={{ on: same(props.on, swatch), auto: same(props.auto, swatch) }}
                  style={{ "--c": swatchColor(swatch) }}
                  onClick={() => props.pick(same(props.auto, swatch) ? null : swatch)}
                />
              );
            }}
          </For>
        )}
      </For>
    </div>
  );
}

const SectionDot = () => <i class="dot srv" style={{ background: "var(--ws)" }} />;

/** A section's menu: a new task in it, its board, its rename and color, and removing it from each server. */
export function SectionMenu(props: { name: string }) {
  const section = () => model.lists.sections.find((shown) => shown.name === props.name);
  const workspaces = () => model.lists.workspaces.filter((workspace) => workspace.name === props.name);
  const [renaming, setRenaming] = createSignal(false);
  const live = (shown: Section, server: string) =>
    shown.groups.reduce((count, group) => count + group.rows.filter((row) => row.at.server === server).length, 0);
  return (
    <Sheet>
      <Show when={section()}>
        {(shown) => (
          <div style={{ display: "contents", "--ws": swatchColor(shown().swatch) }}>
            <SheetHead
              lead={<SectionDot />}
              title={shown().name}
              facts={workspaces().map((workspace) => (
                <span class="fact">
                  {serverLabel(workspace.server)}
                  <Path>{workspace.root}</Path>
                </span>
              ))}
            />
            <Mi slot={<Icon name="plus" />} tap={() => showInSheet({ is: "newtask", section: shown().name })}>
              New task here
            </Mi>
            <Mi
              slot={<Icon name="board" />}
              end={<Chev />}
              tap={() => {
                const top = screen();
                if (top.at === "board" && top.section === shown().name) back();
                else go({ at: "board", section: shown().name });
              }}
            >
              Open board
            </Mi>
            <Show
              when={renaming()}
              fallback={
                <Mi slot={<Icon name="pen" />} tap={() => setRenaming(true)}>
                  Rename…
                </Mi>
              }
            >
              <Rename
                value={shown().name}
                apply={(name) => backend.sectionRename(shown().name, name).then(back)}
                done={() => setRenaming(false)}
              />
            </Show>
            <Flab>color</Flab>
            <Matrix
              on={shown().swatch}
              auto={shown().auto}
              pick={(swatch) => act(backend.sectionColor(shown().name, swatch))}
            />
            <Gap n={2} />
            <Sep />
            <For each={workspaces()}>
              {(workspace) => {
                const count = () => live(shown(), workspace.server);
                const reachable = () => serverView(workspace.server)?.reachable === true;
                const why = () => (!reachable() ? "unreachable" : count() > 0 ? `${count()} live` : null);
                return (
                  <Mi
                    slot={<Icon name="x" />}
                    off={why() !== null}
                    danger={why() === null}
                    end={<Show when={why()}>{(shownWhy) => <Why>{shownWhy()}</Why>}</Show>}
                    tap={() => backend.workspaceRemove(workspace.server, workspace.workspace).then(back, report)}
                  >
                    Remove from {serverLabel(workspace.server)}
                  </Mi>
                );
              }}
            </For>
            <Gap n={2} />
          </div>
        )}
      </Show>
    </Sheet>
  );
}

const ABSOLUTE = /^\//;

/** Adds a workspace on a reachable server; its focus ring, dot and Add take the color chosen. */
export function AddWorkspace() {
  const reachable = () => servers().filter((server) => server.reachable);
  const [server, setServer] = createSignal(reachable()[0]?.name ?? "");
  const [name, setName] = createSignal("");
  const [root, setRoot] = createSignal("");
  const [picked, setPicked] = createSignal<Swatch | null>(null);
  const [problem, setProblem] = createSignal<string | null>(null);
  const [auto] = createResource(() => name().trim(), backend.autoColor);
  const color = () => {
    const swatch = picked() ?? auto();
    return swatch ? swatchColor(swatch) : "var(--text)";
  };
  const add = () => {
    const missing = [
      name().trim() === "" && "name",
      !ABSOLUTE.test(root().trim()) && "an absolute root",
      serverView(server())?.reachable !== true && "a reachable server",
    ].filter(Boolean);
    if (missing.length > 0) return setProblem(`Needs ${missing.join(" and ")}.`);
    backend
      .workspaceAdd(server(), name().trim(), root().trim(), picked())
      .then(back, (error) => setProblem(String(error)));
  };
  return (
    <Sheet style={{ "--ws": color(), "--focus": "var(--ws)" }}>
      <SheetHead
        lead={<SectionDot />}
        title="Add a workspace"
        end={
          <button class="btn ws" onClick={add}>
            Add
          </button>
        }
      />
      <Flab>server</Flab>
      <div class="pick">
        <For each={servers()}>
          {(shown) => (
            <span
              class="chip"
              role="button"
              classList={{ on: shown.name === server(), off: !shown.reachable }}
              onClick={() => shown.reachable && setServer(shown.name)}
            >
              <ServerDot server={shown} />
              {shown.label}
            </span>
          )}
        </For>
      </div>
      <Flab>name</Flab>
      <Field value={name()} input={setName} focus />
      <Flab>root</Flab>
      <Field value={root()} input={setRoot} mono placeholder="/path/on/the/server" enter={add} />
      <Show when={problem()}>{(shown) => <Note err>{shown()}</Note>}</Show>
      <Flab>color</Flab>
      <Matrix on={picked()} auto={auto()} pick={setPicked} />
      <Gap n={2} />
    </Sheet>
  );
}
