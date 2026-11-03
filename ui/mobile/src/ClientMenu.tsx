import { createResource, createSignal, For, Match, Show, Switch } from "solid-js";
import { backend } from "@octopus/shared/backend.ts";
import { aboutCopy, APPEARANCES, NEXT_START } from "@octopus/shared/ClientMenu.tsx";
import { VERSION, ZOOMS, type ShownOffer } from "@octopus/shared/generated.ts";
import { clientStatus, expiry, grouped, inviteCode, isThisClient } from "@octopus/shared/pairing.ts";
import { clock, ticking } from "@octopus/shared/time.ts";
import {
  harnesses,
  me,
  report,
  serverView,
  servers,
  setAgentFlags,
  setAppearance,
  ui,
  zoom,
} from "@octopus/shared/state.ts";
import { shortKey } from "./format";
import { feel } from "./feel";
import { Icon } from "./Icon";
import { go, showInSheet } from "./nav";
import { Chev, Field, Flab, Gap, Gl, Id, Mi, Note, Seg, Sep, ServerDot, Why } from "./parts";
import { Rename } from "./Rename";
import { Sheet, SheetHead } from "./Sheet";
import { notify } from "./Toast";

const toMenu = () => showInSheet({ is: "client" }, "back");

const rtt = (ms: number | null) => `${ms ?? "–"} ms`;

/** Each zoom step ticks, and a step past the end gives the OS's warning instead. */
const step = (to: "in" | "out") => {
  const at = (ZOOMS as readonly number[]).indexOf(ui.prefs.zoom);
  feel((to === "in" ? at === ZOOMS.length - 1 : at === 0) ? "warning" : "select");
  zoom(to);
};

/** Return saves the flags typed, and a refusal shows under the field. */
function AgentFlagsField(props: { harness: string }) {
  const [draft, setDraft] = createSignal(ui.prefs.agent_flags[props.harness] ?? "");
  const [problem, setProblem] = createSignal<string | null>(null);
  const save = () =>
    setAgentFlags(props.harness, draft()).then(
      () => {
        feel("success");
        setProblem(null);
      },
      (error) => {
        feel("error");
        setProblem(String(error));
      },
    );
  return (
    <>
      <Flab>{props.harness}</Flab>
      <Field mono value={draft()} input={setDraft} placeholder="none" enter={save} />
      <Show when={problem()}>{(shown) => <Note err>{shown()}</Note>}</Show>
    </>
  );
}

/** The client menu: this phone's servers, then its appearance, zoom, agent flags and version. */
export function ClientMenu() {
  const { copied, copy: about } = aboutCopy();
  return (
    <Sheet>
      <SheetHead title={me()} />
      <Gl>servers</Gl>
      <For each={servers()}>
        {(server) => (
          <Mi
            class={server.reachable ? undefined : "off"}
            slot={<ServerDot server={server} />}
            end={
              <>
                <Id>{server.reachable ? rtt(server.rtt_ms) : "unreachable"}</Id>
                <Chev />
              </>
            }
            tap={() => showInSheet({ is: "server", name: server.name }, "deeper")}
          >
            {server.label}
          </Mi>
        )}
      </For>
      <Mi slot={<Icon name="plus" />} tap={() => go({ at: "scan" })}>
        Pair a server…
      </Mi>
      <Sep />
      <Mi
        slot={<Icon name="sun" />}
        end={<Seg options={APPEARANCES} value={ui.prefs.appearance} choose={setAppearance} />}
      >
        Appearance
      </Mi>
      <Mi
        slot={<Icon name="zoom" />}
        end={
          <div class="stepper">
            <span role="button" aria-label="Zoom out" onClick={() => step("out")}>
              <Icon name="minus" />
            </span>
            <output>{ui.prefs.zoom}%</output>
            <span role="button" aria-label="Zoom in" onClick={() => step("in")}>
              <Icon name="plus" />
            </span>
          </div>
        }
      >
        Zoom
      </Mi>
      <Show when={harnesses().length > 0}>
        <Gl>agent flags</Gl>
        <For each={harnesses()}>{(harness) => <AgentFlagsField harness={harness} />}</For>
        <Note>{NEXT_START}</Note>
      </Show>
      <Sep />
      <Mi slot={<Icon name="info" />} end={<Why>{copied() ? "copied" : VERSION}</Why>} tap={about}>
        About octopus
      </Mi>
      <Gap n={2} />
    </Sheet>
  );
}

type Mode = "main" | "rename" | "forget" | ShownOffer;

/** A server's screen in the sheet: its reachability and key, its pairing code, its paired clients, rename and forget. */
export function ServerMenu(props: { name: string }) {
  const server = () => serverView(props.name);
  const now = ticking();
  const [mode, setMode] = createSignal<Mode>("main");
  const offer = () => {
    const shown = mode();
    return typeof shown === "string" ? null : shown;
  };
  const [clients, { refetch }] = createResource(
    () => (server()?.reachable ? props.name : false),
    backend.serverClients,
  );
  const facts = () => {
    const shown = server();
    if (shown === undefined) return [];
    const reach = shown.reachable
      ? `reachable · ${rtt(shown.rtt_ms)}`
      : `unreachable since ${clock(shown.since, "h23")}`;
    return [reach, shown.key === null ? "local socket" : `key ${shortKey(shown.key)}`];
  };
  const showCode = () => backend.serverOffer(props.name).then(setMode, report);
  const copyInvite = (invite: string) =>
    navigator.clipboard.writeText(invite).then(() => notify("Copied the pairing invite"), report);
  const shareInvite = (invite: string) =>
    navigator.share
      ? navigator.share({ text: invite }).catch((error: Error) => {
          if (error.name !== "AbortError") report(error);
        })
      : copyInvite(invite);
  const revoke = (client: string) =>
    backend.serverRevoke(props.name, client).then(() => {
      feel("success");
      refetch();
    }, report);
  const rename = (name: string) =>
    backend.serverRename(props.name, name).then(() => showInSheet({ is: "server", name }, "here"));
  const forget = () =>
    backend.serverForget(props.name).then(() => {
      feel("success");
      toMenu();
    }, report);
  const head = (lines: string[], up: () => void) => <SheetHead title={props.name} facts={lines} back={up} />;
  return (
    <Sheet>
      <Switch>
        <Match when={offer()}>
          {(offered) => (
            <>
              {head([expiry(offered().offer, now())], () => setMode("main"))}
              <div class="qr" innerHTML={offered().qr} />
              <Gap n={3} />
              <div class="cmd">
                <span class="name">{grouped(inviteCode(offered().offer))}</span>
              </div>
              <Gap n={3} />
              <div class="pair2">
                <button class="secondary" onClick={() => copyInvite(offered().offer.invite)}>
                  <Icon name="copy" />
                  Copy invite
                </button>
                <button class="secondary" onClick={() => shareInvite(offered().offer.invite)}>
                  <Icon name="share" />
                  Share
                </button>
              </div>
              <Gap n={2} />
            </>
          )}
        </Match>
        <Match when={true}>
          {head(facts(), toMenu)}
          <Mi
            slot={<Icon name="qr" />}
            off={!server()?.reachable}
            end={server()?.reachable ? <Chev /> : <Why>unreachable</Why>}
            tap={showCode}
          >
            Show pairing code
          </Mi>
          <Show when={mode() !== "forget"}>
            <Gl>paired clients</Gl>
            <Show
              when={server()?.reachable && !clients.error && clients()}
              fallback={
                <div class="empty">
                  {!server()?.reachable
                    ? "unknown while unreachable"
                    : clients.error
                      ? String(clients.error)
                      : "listing…"}
                </div>
              }
            >
              {(listed) => (
                <For each={listed()}>
                  {(client) => (
                    <Mi
                      slot={<i class={client.connected || isThisClient(client) ? "dot srv" : "dot idle"} />}
                      end={
                        <>
                          <Why>{clientStatus(client, now())}</Why>
                          <Show when={!isThisClient(client)}>
                            <button class="btn danger" onClick={() => revoke(client.name)}>
                              Revoke
                            </button>
                          </Show>
                        </>
                      }
                    >
                      {client.name}
                    </Mi>
                  )}
                </For>
              )}
            </Show>
          </Show>
          <Sep />
          <Show
            when={mode() === "rename"}
            fallback={
              <Mi slot={<Icon name="pen" />} tap={() => setMode("rename")}>
                Rename…
              </Mi>
            }
          >
            <Rename value={props.name} apply={rename} done={() => setMode("main")} />
          </Show>
          <Show
            when={mode() === "forget"}
            fallback={
              <Mi
                slot={<Icon name="x" />}
                danger
                tap={() => {
                  feel("warning");
                  setMode("forget");
                }}
              >
                Forget server
              </Mi>
            }
          >
            <Gap n={2} />
            <div class="confirm">
              <span>
                Forget {props.name}? Its tasks leave this client's lists; they keep running on {props.name}. Pairing
                again needs a new code.
              </span>
              <div class="row2">
                <button class="btn quiet" onClick={() => setMode("main")}>
                  Cancel
                </button>
                <button class="btn danger" onClick={forget}>
                  Forget
                </button>
              </div>
            </div>
          </Show>
          <Gap n={2} />
        </Match>
      </Switch>
    </Sheet>
  );
}
