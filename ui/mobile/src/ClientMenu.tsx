import { createResource, createSignal, For, Match, Show, Switch } from "solid-js";
import { backend } from "@octopus/shared/backend.ts";
import { aboutCopy, APPEARANCES } from "@octopus/shared/ClientMenu.tsx";
import { VERSION, ZOOMS, type ShownOffer } from "@octopus/shared/generated.ts";
import { clientStatus, clock, expiry, grouped, inviteCode, ticking } from "@octopus/shared/SidebarControls.tsx";
import { me, report, serverView, servers, setAppearance, ui, zoom } from "@octopus/shared/state.ts";
import { host, shortKey } from "./format";
import { feel } from "./feel";
import { Icon } from "./Icon";
import { go, showInSheet } from "./nav";
import { Chev, Gap, Gl, Id, Mi, Seg, Sep, ServerDot, Why } from "./parts";
import { Rename } from "./Rename";
import { Sheet, SheetHead } from "./Sheet";
import { notify } from "./Toast";

const toMenu = () => showInSheet({ is: "client" }, "back");

/** A private IPv4 address, the one a server's header names as its LAN. */
const LAN = /^(10\.|192\.168\.|172\.(1[6-9]|2\d|3[01])\.)/;

const rtt = (ms: number | null) => `${ms ?? "–"} ms`;

/** Each zoom step ticks, and a step past the end gives the OS's warning instead. */
const step = (to: "in" | "out") => {
  const at = (ZOOMS as readonly number[]).indexOf(ui.prefs.zoom);
  feel((to === "in" ? at === ZOOMS.length - 1 : at === 0) ? "warning" : "select");
  zoom(to);
};

/** The client menu: this phone's servers, then its appearance, zoom and version. */
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
    const key = shown.key === null ? "local socket" : `key ${shortKey(shown.key)}`;
    const lan = shown.addresses.find((address) => LAN.test(address)) ?? shown.addresses[0];
    return [reach, lan === undefined ? key : `${key} · lan ${host(lan)}`];
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
                      slot={<i class={client.connected || client.name === me() ? "dot srv" : "dot idle"} />}
                      end={
                        <>
                          <Why>{clientStatus(client, now())}</Why>
                          <Show when={client.name !== me()}>
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
