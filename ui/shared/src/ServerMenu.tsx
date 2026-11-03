import { createResource, createSignal, For, onCleanup, Show } from "solid-js";
import { backend } from "./backend";
import type { Offer, ShownOffer } from "./generated";
import { mac } from "./commands";
import { closeMenu, closing, Head, Item, Sep } from "./Menu";
import { clientStatus, expiry, grouped, inviteCode, isThisClient } from "./pairing";
import { startRename } from "./SidebarControls";
import { isLocal, report, serverLabel, serverView, showSidebar } from "./state";
import { clock, ticking } from "./time";
import { copy, toast } from "./Toast";

const copyKey = mac ? "⌘C" : "Ctrl+C";

export function ServerMenu(props: { name: string }) {
  const server = () => serverView(props.name);
  const label = serverLabel(props.name);
  const now = ticking();
  const [mode, setMode] = createSignal<"main" | "forget" | ShownOffer>("main");
  const offer = () => {
    const shown = mode();
    return typeof shown === "string" ? null : shown;
  };
  const copyInvite = (offered: Offer) => copy(offered.invite, "the pairing invite").then(closeMenu, report);
  const copyShortcut = (event: KeyboardEvent) => {
    const offered = offer();
    if (offered === null || event.key !== "c" || !(mac ? event.metaKey : event.ctrlKey)) return;
    event.preventDefault();
    copyInvite(offered.offer);
  };
  document.addEventListener("keydown", copyShortcut);
  onCleanup(() => document.removeEventListener("keydown", copyShortcut));
  const [clients, { refetch }] = createResource(
    () => (server()?.reachable ? props.name : false),
    backend.serverClients,
  );
  const showCode = () => backend.serverOffer(props.name).then(setMode, report);
  const revoke = (client: string) =>
    backend.serverRevoke(props.name, client).then(() => {
      refetch();
      toast(
        <>
          Revoked <b>{client}</b> on {label}
        </>,
      );
    }, report);
  const forget = () =>
    backend.serverForget(props.name).then(() => {
      closeMenu();
      toast(
        <>
          Forgot <b>{label}</b>
        </>,
      );
    }, report);

  return (
    <>
      <Head
        title={props.name}
        lines={[
          server()?.relay ?? "local socket",
          server()?.reachable
            ? `reachable · ${server()?.rtt_ms ?? "–"} ms`
            : `unreachable since ${clock(server()?.since ?? now())}`,
        ]}
      />
      <Show when={offer()}>
        {(offered) => (
          <>
            <div class="code">
              <div class="qr" innerHTML={offered().qr} />
              <div class="digits">{grouped(inviteCode(offered().offer))}</div>
              <div class="exp">{expiry(offered().offer, now())}</div>
            </div>
            <Item kbd={copyKey} run={() => copyInvite(offered().offer)}>
              Copy invite
            </Item>
          </>
        )}
      </Show>
      <Show when={mode() === "forget"}>
        <div class="confirm">
          <span>
            Forget {label}? Its tasks leave this client's lists; they keep running on {label}.{" "}
            {isLocal(props.name) ? "It returns when the app next starts." : "Pairing again needs a new invite."}
          </span>
          <div class="row">
            <button class="btn" onClick={() => setMode("main")}>
              Cancel
            </button>
            <button class="btn danger" onClick={forget}>
              Forget
            </button>
          </div>
        </div>
      </Show>
      <Show when={mode() === "main"}>
        <Item run={showCode} why={server()?.reachable ? undefined : "unreachable"}>
          Show pairing code
        </Item>
        <Sep />
        <div class="mh label">
          <span>Paired clients</span>
        </div>
        <Show
          when={server()?.reachable && !clients.error && (clients() ?? []).length > 0}
          fallback={
            <div class="mh label">
              {!server()?.reachable
                ? "unknown while unreachable"
                : clients.error
                  ? String(clients.error)
                  : clients.loading
                    ? "listing…"
                    : "only this client, over the local socket"}
            </div>
          }
        >
          <For each={clients()}>
            {(client) => (
              <div class="client">
                <span class="dot" classList={{ idle: !client.connected }} />
                <span class="name">{client.name}</span>
                <span class="mono">{clientStatus(client, now())}</span>
                <Show when={!isThisClient(client)}>
                  <button class="mini" onClick={() => revoke(client.name)}>
                    Revoke
                  </button>
                </Show>
              </div>
            )}
          </For>
        </Show>
        <Sep />
        <Item
          run={closing(() => {
            showSidebar(true);
            startRename("server", props.name);
          })}
        >
          Rename…
        </Item>
        <Item danger run={() => setMode("forget")}>
          Forget server
        </Item>
      </Show>
    </>
  );
}
