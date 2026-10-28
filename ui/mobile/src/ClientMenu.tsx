import { createResource, createSignal, For, Match, Show, Switch } from "solid-js";
import { backend } from "@octopus/shared/backend.ts";
import { aboutCopy, APPEARANCES } from "@octopus/shared/ClientMenu.tsx";
import { VERSION, type ShownOffer } from "@octopus/shared/generated.ts";
import { clientStatus, clock, expiry, grouped, inviteCode, ticking, toast } from "@octopus/shared/SidebarControls.tsx";
import { me, report, serverView, servers, setAppearance, setMe, ui, zoom } from "@octopus/shared/state.ts";
import { phone, scanInvite } from "./phone";
import { closeMenu, menuPage, openMenu } from "./screen";
import { Sheet, SheetHead } from "./Sheet";
import { Rename } from "./Rename";

const toMenu = () => openMenu({ show: "menu" });

function Items() {
  const { copied, copy: about } = aboutCopy();
  return (
    <>
      <SheetHead title={me()} />
      <div class="label">Servers</div>
      <For each={servers()}>
        {(server) => (
          <button class="si" onClick={() => openMenu({ show: "server", name: server.name })}>
            <span class="dot" classList={{ off: !server.reachable }} />
            <span class="name">{server.label}</span>
            <span class="rtt">{server.reachable ? `${server.rtt_ms ?? "–"} ms` : "unreachable"}</span>
            <span class="chev">›</span>
          </button>
        )}
      </For>
      <button class="si" onClick={() => openMenu({ show: "pair" })}>
        <span class="glyph">+</span>Pair a server…
      </button>
      <div class="sep" />
      <div class="srow">
        Appearance
        <div class="seg">
          <For each={APPEARANCES}>
            {([appearance, label]) => (
              <button classList={{ on: ui.prefs.appearance === appearance }} onClick={() => setAppearance(appearance)}>
                {label}
              </button>
            )}
          </For>
        </div>
      </div>
      <div class="srow">
        Zoom
        <span class="stepper">
          <button class="step" aria-label="Zoom out" onClick={() => zoom("out")}>
            −
          </button>
          <output>{ui.prefs.zoom}%</output>
          <button class="step" aria-label="Zoom in" onClick={() => zoom("in")}>
            +
          </button>
        </span>
      </div>
      <div class="sep" />
      <button class="si" onClick={about}>
        About octopus<span class="why">{copied() ? "copied" : VERSION}</span>
      </button>
    </>
  );
}

function ServerPage(props: { name: string }) {
  const server = () => serverView(props.name);
  const now = ticking();
  const [mode, setMode] = createSignal<"main" | "rename" | "forget" | ShownOffer>("main");
  const offer = () => {
    const shown = mode();
    return typeof shown === "string" ? null : shown;
  };
  const [clients, { refetch }] = createResource(
    () => (server()?.reachable ? props.name : false),
    backend.serverClients,
  );
  const showCode = () => backend.serverOffer(props.name).then(setMode, report);
  const copyInvite = (invite: string) =>
    navigator.clipboard.writeText(invite).then(() => toast("Copied the pairing invite"), report);
  const revoke = (client: string) => backend.serverRevoke(props.name, client).then(refetch, report);
  const rename = (name: string) =>
    backend.serverRename(props.name, name).then(() => openMenu({ show: "server", name }));
  const forget = () => backend.serverForget(props.name).then(toMenu, report);
  return (
    <>
      <SheetHead
        back={toMenu}
        title={
          <Show when={mode() === "rename"} fallback={props.name}>
            <Rename value={props.name} apply={rename} done={() => setMode("main")} />
          </Show>
        }
        lines={[
          server()?.relay ?? "",
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
            <button class="si" onClick={() => copyInvite(offered().offer.invite)}>
              Copy invite
            </button>
          </>
        )}
      </Show>
      <Show when={mode() === "forget"}>
        <div class="confirm">
          <span>
            Forget {props.name}? Its tasks leave this phone's list; they keep running on {props.name}. Pairing again
            needs a new invite.
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
      <Show when={mode() === "main" || mode() === "rename"}>
        <button
          class="si"
          aria-disabled={server()?.reachable ? undefined : "true"}
          onClick={() => server()?.reachable && showCode()}
        >
          Show pairing code
          <Show when={!server()?.reachable}>
            <span class="why">unreachable</span>
          </Show>
        </button>
        <div class="sep" />
        <div class="label">Paired clients</div>
        <Show
          when={server()?.reachable && !clients.error && clients()}
          fallback={
            <div class="label">
              {!server()?.reachable ? "unknown while unreachable" : clients.error ? String(clients.error) : "listing…"}
            </div>
          }
        >
          {(listed) => (
            <For each={listed()}>
              {(client) => (
                <div class="client">
                  <span class="dot" classList={{ idle: !client.connected }} />
                  <span class="name">{client.name}</span>
                  <span class="mono">{clientStatus(client, now())}</span>
                  <Show when={client.name !== me()}>
                    <button class="revoke" onClick={() => revoke(client.name)}>
                      Revoke
                    </button>
                  </Show>
                </div>
              )}
            </For>
          )}
        </Show>
        <div class="sep" />
        <button class="si" onClick={() => setMode("rename")}>
          Rename…
        </button>
        <button class="si danger" onClick={() => setMode("forget")}>
          Forget server
        </button>
      </Show>
    </>
  );
}

function PairPage() {
  const [named] = createResource(phone.named);
  const [invite, setInvite] = createSignal("");
  const [name, setName] = createSignal(me());
  const [problem, setProblem] = createSignal<string | null>(null);
  const [pairing, setPairing] = createSignal(false);
  const naming = () => named() === false;
  const complete = () => invite().trim() !== "" && (!naming() || name().trim() !== "") && !pairing();
  const scanned = () =>
    scanInvite().then(
      (text) => {
        setInvite(text);
        setProblem(null);
      },
      (error: { message: string }) => setProblem(error.message),
    );
  const pair = (event: SubmitEvent) => {
    event.preventDefault();
    if (!complete()) return;
    setPairing(true);
    setProblem(null);
    phone
      .pair(invite().trim(), naming() ? name().trim() : null)
      .then(
        () => {
          if (naming()) setMe(name().trim());
          toMenu();
        },
        (error) => setProblem(String(error)),
      )
      .finally(() => setPairing(false));
  };
  return (
    <form class="pair" onSubmit={pair}>
      <SheetHead back={toMenu} title="Pair a server" />
      <label class="field">
        <span>Invite</span>
        <textarea
          class="mono"
          rows={3}
          spellcheck={false}
          autocapitalize="off"
          autocorrect="off"
          placeholder="octopus://…"
          value={invite()}
          onInput={(event) => setInvite(event.currentTarget.value)}
        />
      </label>
      <button type="button" class="btn" onClick={scanned}>
        Scan QR
      </button>
      <Show when={naming()}>
        <label class="field">
          <span>This phone's name</span>
          <input
            spellcheck={false}
            autocorrect="off"
            value={name()}
            onInput={(event) => setName(event.currentTarget.value)}
          />
        </label>
      </Show>
      <Show when={problem()}>{(shown) => <div class="err">{shown()}</div>}</Show>
      <button type="submit" class="btn go" disabled={!complete()}>
        {pairing() ? "Pairing…" : "Pair"}
      </button>
    </form>
  );
}

const shownServer = () => {
  const page = menuPage();
  return page?.show === "server" ? page.name : null;
};

export function ClientMenu() {
  return (
    <Sheet shown={menuPage() !== null} close={closeMenu}>
      <Switch>
        <Match when={menuPage()?.show === "menu"}>
          <Items />
        </Match>
        <Match when={menuPage()?.show === "pair"}>
          <PairPage />
        </Match>
        <Match when={shownServer()} keyed>
          {(name) => <ServerPage name={name} />}
        </Match>
      </Switch>
    </Sheet>
  );
}
