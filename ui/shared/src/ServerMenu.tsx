import { createSignal, For, Show } from "solid-js";
import { backend } from "./backend";
import type { PairedAccount, ShownOffer } from "./generated";
import { closeMenu, closing, Confirm, copyKey, copyOnShortcut, Head, Item, Sep } from "./Menu";
import {
  accountLabel,
  clientStatus,
  expiry,
  firstWhere,
  grouped,
  inviteCode,
  isThisAccount,
  isThisClient,
  pairedAccounts,
  shortAccount,
} from "./pairing";
import { startRename } from "./SidebarControls";
import { isLocal, report, serverLabel, serverView, showSidebar } from "./state";
import { clock, ticking } from "./time";
import { copy, toast } from "./Toast";

export function ServerMenu(props: { name: string }) {
  const server = () => serverView(props.name);
  const label = serverLabel(props.name);
  const now = ticking();
  const [mode, setMode] = createSignal<"main" | "forget" | ShownOffer>("main");
  const offer = () => {
    const shown = mode();
    return typeof shown === "string" ? null : shown;
  };
  const copyInvite = (offered: ShownOffer) => copy(offered.offer.invite, "the pairing invite").then(closeMenu, report);
  copyOnShortcut(offer, copyInvite);
  const { listed, note, refetch } = pairedAccounts(props.name);
  const showCode = () => backend.serverOffer(props.name).then(setMode, report);
  const revoke = (account: PairedAccount) =>
    backend.serverRevoke(props.name, account.account).then(() => {
      refetch();
      toast(
        <>
          Revoked account <b>{shortAccount(account.account)}</b> on {label}
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
              <div class="exp">{expiry(offered().offer.expires_at, now(), "one pairing")}</div>
            </div>
            <Item kbd={copyKey} run={() => copyInvite(offered())}>
              Copy invite
            </Item>
          </>
        )}
      </Show>
      <Show when={mode() === "forget"}>
        <Confirm act="Forget" run={forget} cancel={() => setMode("main")}>
          Forget {label}? Its tasks leave this client's lists; they keep running on {label}.{" "}
          {isLocal(props.name) ? "It returns when the app next starts." : "Pairing again needs a new invite."}
        </Confirm>
      </Show>
      <Show when={mode() === "main"}>
        <Item run={showCode} why={server()?.reachable ? undefined : "unreachable"}>
          Show pairing code
        </Item>
        <Sep />
        <Show
          when={listed()?.length}
          fallback={
            <>
              <div class="mh label">
                <span>Paired accounts</span>
              </div>
              <div class="mh label">{note()}</div>
            </>
          }
        >
          <For each={listed()}>
            {(paired) => (
              <>
                <div class="client">
                  <span class="mono">{accountLabel(paired)}</span>
                  <Show when={!isThisAccount(paired)}>
                    <button class="mini" onClick={() => revoke(paired)}>
                      Revoke
                    </button>
                  </Show>
                </div>
                <For each={firstWhere(paired.devices, isThisClient)}>
                  {(client) => (
                    <div class="client">
                      <span class="dot" classList={{ idle: !client.connected && !client.hosts }} />
                      <span class="name">{client.name}</span>
                      <span class="mono">{clientStatus(client, now())}</span>
                    </div>
                  )}
                </For>
              </>
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
