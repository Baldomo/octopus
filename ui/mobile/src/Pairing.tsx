import logo from "../../../crates/mobile/icons/icon.png?url";
import { readText } from "@tauri-apps/plugin-clipboard-manager";
import { createEffect, createResource, createSignal, on, onCleanup, Show, type JSX } from "solid-js";
import { backend } from "@octopus/shared/backend.ts";
import { isLink } from "@octopus/shared/pairing.ts";
import { account, act, me, report, setMe } from "@octopus/shared/state.ts";
import { shortKey } from "./format";
import { feel } from "./feel";
import { Icon } from "./Icon";
import { back, go, home, openSheet, type Reading, screen } from "./nav";
import { Bar, Circle, Field, Flab, Gap, Id, Mi, Note, Para, Primary, Secondary } from "./parts";
import { phone, scanInvite, stopScanning } from "./phone";
import { Sheet, SheetHead } from "./Sheet";

/** The name a phone that has never paired takes on its first pairing; a named phone keeps its own. */
const [named, { refetch: recheckNamed }] = createResource(phone.named);
const [phoneName, setPhoneName] = createSignal<string | null>(null);
const nameToTake = () => (named() === false ? (phoneName() ?? me()).trim() : null);

/** Whether the camera shows behind the page, which then draws nothing over it but the finder's frame. */
export const [scanning, setScanning] = createSignal(false);

const paste = (reading: Reading) =>
  readText().then((text) => {
    const invite = text.trim();
    if (invite === "") return report(`The clipboard holds no ${reading}.`);
    openSheet({ is: "pair", invite, reading });
  }, report);

/** Counts the scans asked for, so reading again restarts the camera under the sheet it closes. */
const [scans, rescan] = createSignal(0);

const PasteInvite = (props: { reading: Reading }) => (
  <Secondary tap={() => paste(props.reading)}>
    <Icon name="paste" />
    Paste invite
  </Secondary>
);

export function Welcome() {
  return (
    <>
      <div class="fill" />
      <img class="applogo" src={logo} alt="" />
      <Gap n={4} />
      <div class="h1">Pair this phone</div>
      <Gap n={2} />
      <Para>
        octopus runs on computers you own. On one of them, open a server's menu and choose <b>Show pairing code</b>, or
        the client menu's <b>Link a device…</b> to join its account.
      </Para>
      <Show when={named() === false}>
        <Gap n={5} />
        <Flab>this phone's name</Flab>
        <Field value={phoneName() ?? me()} input={setPhoneName} />
      </Show>
      <div class="fill" />
      <Primary tap={() => go({ at: "scan", reading: "invite" })}>
        <Icon name="cam" />
        Scan invite
      </Primary>
      <Gap n={2} />
      <PasteInvite reading="invite" />
      <Gap n={2} />
    </>
  );
}

/** How the scanner rejects the scan that leaving the screen stops. */
const CANCELLED = "cancelled";

export function Scan(props: { reading: Reading }) {
  const [unscanned, setUnscanned] = createSignal<string | null>(null);
  /** One scan at a time: a paste over a running scan leaves it running for "Scan again" to find. */
  const start = () => {
    if (scanning()) return;
    setScanning(true);
    setUnscanned(null);
    scanInvite()
      .then(
        (invite) => openSheet({ is: "pair", invite, reading: props.reading }),
        (error: { message: string }) => setUnscanned(error.message === CANCELLED ? null : error.message),
      )
      .finally(() => setScanning(false));
  };
  createEffect(on(scans, start));
  onCleanup(() => scanning() && act(stopScanning()));
  return (
    <>
      <Bar>
        <Circle icon="close" label="Close" tap={back} />
        <div class="ttl">
          <b>Scan invite</b>
        </div>
      </Bar>
      <Gap n={5} />
      <div class="finder" />
      <Gap n={4} />
      <Para center>
        {unscanned() === null
          ? "Point the camera at the invite or link on your computer's screen."
          : `${unscanned()} Paste the invite or link instead.`}
      </Para>
      <div class="fill" />
      <PasteInvite reading={props.reading} />
      <Gap n={2} />
    </>
  );
}

/** Reading an invite again goes back to the camera, which a pasted invite never left for. */
function readAgain(reading: Reading) {
  if (screen().at !== "scan") return go({ at: "scan", reading });
  back();
  rescan((count) => count + 1);
}

const REFUSED: Record<Reading, { title: string; facts: string; nothing: string; anew: () => JSX.Element }> = {
  link: {
    title: "Join an account",
    facts: "one-time link",
    nothing: "This phone joined nothing",
    anew: () => (
      <>
        Make a new one on a device of the account: its client menu's <b>Link a device…</b>, or{" "}
        <b>octopus account link</b>.
      </>
    ),
  },
  invite: {
    title: "Pair a server",
    facts: "one-time invite",
    nothing: "This phone paired nothing",
    anew: () => (
      <>
        Show a new one on the server: its server menu's <b>Show pairing code</b>, or <b>octopus pairing offer</b>.
      </>
    ),
  },
};

/** Why an invite paired nothing or a link joined nothing, in full, and where to get a fresh one. */
function Refused(props: { problem: string; reading: Reading }) {
  const refused = () => REFUSED[props.reading];
  return (
    <>
      <SheetHead title={refused().title} facts={[refused().facts]} />
      <Note err>{refused().nothing}</Note>
      <Para>{props.problem}</Para>
      <Gap n={2} />
      <Para>{refused().anew()}</Para>
      <Gap n={3} />
      <Primary tap={() => readAgain(props.reading)}>Scan again</Primary>
      <Gap n={2} />
    </>
  );
}

/** Sends a pairing or a join under the name this phone takes, then goes home, or shows the refusal. */
function sending() {
  const [busy, setBusy] = createSignal(false);
  const [refusal, setRefusal] = createSignal<string | null>(null);
  const send = <T,>(request: (taking: string | null) => Promise<T>, after?: (result: T) => Promise<unknown>) => {
    setBusy(true);
    const taking = nameToTake();
    request(taking)
      .then(async (result) => {
        if (taking !== null) setMe(taking);
        recheckNamed();
        await after?.(result);
        feel("success");
        home();
      })
      .catch((error) => {
        feel("error");
        setRefusal(String(error));
      })
      .finally(() => setBusy(false));
  };
  return { busy, refusal, send };
}

const IN_A_SHARED_ACCOUNT =
  "This phone is in an account with other devices. Leave it first: the client menu's Leave account.";

/** A phone joins only while it is its account's one device. */
function JoinSheet(props: { link: string }) {
  const [linked] = createResource(() => props.link, phone.linkAccount);
  const { busy, refusal, send } = sending();
  const problem = () =>
    refusal() ??
    (linked.error ? String(linked.error) : null) ??
    (linked() === account().id ? "Already in this account." : null) ??
    (account().member && account().devices.length > 1 ? IN_A_SHARED_ACCOUNT : null);
  const join = () => send((taking) => phone.join(props.link, taking));
  return (
    <Show
      when={problem()}
      fallback={
        <>
          <SheetHead title="Join an account" facts={[`account ${linked() ?? ""}`]} />
          <Show when={named() === false}>
            <Flab>name on this phone</Flab>
            <Field value={phoneName() ?? me()} input={setPhoneName} />
            <Gap n={2} />
          </Show>
          <Para>This phone takes the account's servers and keeps its own.</Para>
          <Gap n={3} />
          <Primary tap={join} off={busy() || nameToTake() === ""}>
            {busy() ? "Joining…" : "Join"}
          </Primary>
          <Gap n={2} />
        </>
      }
    >
      {(shown) => <Refused problem={shown()} reading="link" />}
    </Show>
  );
}

/** A link joins an account; an invite pairs a server; anything else refuses as what the user was reading for. */
export function PairSheet(props: { invite: string; reading: Reading }) {
  return (
    <Show when={isLink(props.invite) || props.reading === "link"} fallback={<PairServerSheet invite={props.invite} />}>
      <Sheet>
        <JoinSheet link={props.invite} />
      </Sheet>
    </Show>
  );
}

/** The key of the server an invite pairs, and the name this phone keeps for it. */
function PairServerSheet(props: { invite: string }) {
  const [inviteKey] = createResource(() => props.invite, phone.inviteKey);
  const [name, setName] = createSignal("");
  const { busy, refusal, send } = sending();
  const pair = () =>
    send(
      (taking) => phone.pair(props.invite, taking),
      async (paired) => {
        const kept = name().trim();
        if (kept !== "" && kept !== paired) await backend.serverRename(paired, kept).catch(report);
      },
    );
  return (
    <Sheet>
      <Show
        when={refusal() ?? (inviteKey.error ? String(inviteKey.error) : null)}
        fallback={
          <Show when={inviteKey()}>
            {(key) => (
              <>
                <SheetHead title="Pair a server" facts={["one-time invite"]} />
                <Flab>name on this phone</Flab>
                <Field value={name()} input={setName} placeholder="the name it pairs with" />
                <Gap n={2} />
                <Mi slot={<Icon name="key" />} end={<Id>ed25519 {shortKey(key())}</Id>}>
                  key
                </Mi>
                <Gap n={2} />
                <Primary tap={pair} off={busy() || nameToTake() === ""}>
                  {busy() ? "Pairing…" : "Pair"}
                </Primary>
                <Gap n={2} />
              </>
            )}
          </Show>
        }
      >
        {(shown) => <Refused problem={shown()} reading="invite" />}
      </Show>
    </Sheet>
  );
}
