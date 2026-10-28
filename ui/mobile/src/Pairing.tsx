import logo from "../../../crates/mobile/icons/icon.png?url";
import { readText } from "@tauri-apps/plugin-clipboard-manager";
import { createEffect, createResource, createSignal, on, onCleanup, Show } from "solid-js";
import { backend } from "@octopus/shared/backend.ts";
import { me, report, setMe } from "@octopus/shared/state.ts";
import { shortKey } from "./format";
import { Icon } from "./Icon";
import { back, go, home, openSheet, screen } from "./nav";
import { Bar, Circle, Field, Flab, Gap, Id, Mi, Note, Para, Primary, Secondary } from "./parts";
import { phone, scanInvite, stopScanning } from "./phone";
import { Sheet, SheetHead } from "./Sheet";

/** The name a phone that has never paired takes on its first pairing; a named phone keeps its own. */
const [named, { refetch: recheckNamed }] = createResource(phone.named);
const [phoneName, setPhoneName] = createSignal<string | null>(null);
const nameToTake = () => (named() === false ? (phoneName() ?? me()).trim() : null);

/** Whether the camera shows behind the page, which then draws nothing over it but the finder's frame. */
export const [scanning, setScanning] = createSignal(false);

const paste = () =>
  readText().then((text) => {
    const invite = text.trim();
    if (invite === "") return report("The clipboard holds no invite.");
    openSheet({ is: "pair", invite });
  }, report);

/** Counts the scans asked for, so reading again restarts the camera under the sheet it closes. */
const [scans, rescan] = createSignal(0);

const PasteInvite = () => (
  <Secondary tap={paste}>
    <Icon name="paste" />
    Paste invite
  </Secondary>
);

/** First run: the phone's name, which each server lists it by, and the two ways to read an invite. */
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
        run <b>octopus pairing offer</b>.
      </Para>
      <Show when={named() === false}>
        <Gap n={5} />
        <Flab>this phone's name</Flab>
        <Field value={phoneName() ?? me()} input={setPhoneName} />
      </Show>
      <div class="fill" />
      <Primary tap={() => go({ at: "scan" })}>
        <Icon name="cam" />
        Scan invite
      </Primary>
      <Gap n={2} />
      <PasteInvite />
      <Gap n={2} />
    </>
  );
}

/** How the scanner rejects the scan that leaving the screen stops. */
const CANCELLED = "cancelled";

/** The camera, dark around its finder, filling the field from the first QR code that is an invite. */
export function Scan() {
  /** One scan at a time: a paste over a running scan leaves it running for "Scan again" to find. */
  const start = () => {
    if (scanning()) return;
    setScanning(true);
    scanInvite()
      .then(
        (invite) => openSheet({ is: "pair", invite }),
        (error: { message: string }) => {
          if (error.message !== CANCELLED) report(error.message);
        },
      )
      .finally(() => setScanning(false));
  };
  createEffect(on(scans, start));
  onCleanup(() => scanning() && stopScanning().catch(report));
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
      <Para center>Point the camera at the invite on your computer's screen.</Para>
      <div class="fill" />
      <PasteInvite />
      <Gap n={2} />
    </>
  );
}

/** Reading an invite again goes back to the camera, which a pasted invite never left for. */
function readAgain() {
  if (screen().at !== "scan") return go({ at: "scan" });
  back();
  rescan((count) => count + 1);
}

function Refused(props: { problem: string }) {
  return (
    <>
      <SheetHead title="Pair a server" facts={["one-time invite"]} />
      <Note err>{props.problem}</Note>
      <Para>
        Show a new one on the server: its server menu's <b>Show pairing code</b>, or <b>octopus pairing offer</b>.
      </Para>
      <Gap n={3} />
      <Primary tap={readAgain}>Scan again</Primary>
      <Gap n={2} />
    </>
  );
}

/** The key of the server an invite pairs, and the name this phone keeps for it. */
export function PairSheet(props: { invite: string }) {
  const [inviteKey] = createResource(() => props.invite, phone.inviteKey);
  const [name, setName] = createSignal("");
  const [pairing, setPairing] = createSignal(false);
  const [problem, setProblem] = createSignal<string | null>(null);
  const pair = () => {
    setPairing(true);
    const taking = nameToTake();
    phone
      .pair(props.invite, taking)
      .then(async (paired) => {
        if (taking !== null) setMe(taking);
        recheckNamed();
        const kept = name().trim();
        if (kept !== "" && kept !== paired) await backend.serverRename(paired, kept).catch(report);
        home();
      })
      .catch((error) => setProblem(String(error)))
      .finally(() => setPairing(false));
  };
  return (
    <Sheet>
      <Show
        when={problem() ?? (inviteKey.error ? String(inviteKey.error) : null)}
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
                <Primary tap={pair} off={pairing() || nameToTake() === ""}>
                  {pairing() ? "Pairing…" : "Pair"}
                </Primary>
                <Gap n={2} />
              </>
            )}
          </Show>
        }
      >
        {(shown) => <Refused problem={shown()} />}
      </Show>
    </Sheet>
  );
}
