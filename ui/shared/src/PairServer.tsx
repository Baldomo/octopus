import { createSignal } from "solid-js";
import { backend } from "./backend";
import { Actions, TextField } from "./Dialog";
import { report, showModal } from "./state";

function SendText(props: {
  label: string;
  placeholder: string;
  submit: string;
  send: (text: string) => Promise<unknown>;
}) {
  const [text, setText] = createSignal("");
  const complete = () => text().trim() !== "";
  const send = (event: SubmitEvent) => {
    event.preventDefault();
    if (!complete()) return;
    props.send(text().trim()).then(() => showModal(null), report);
  };
  return (
    <form class="pane dialog" onSubmit={send}>
      <TextField label={props.label} placeholder={props.placeholder} focus value={text()} change={setText} />
      <Actions submit={props.submit} complete={complete()} />
    </form>
  );
}

export const PairServer = () => (
  <SendText label="invite" placeholder="from octopus pairing offer" submit="Pair" send={backend.serverPair} />
);

export const JoinAccount = () => (
  <SendText label="link" placeholder="from Link a device… on another device" submit="Join" send={backend.accountJoin} />
);
