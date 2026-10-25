import { createSignal } from "solid-js";
import { backend } from "./backend";
import { Actions, TextField } from "./Dialog";
import { report, showModal } from "./state";

/** Pairs with a server on another machine through the code its `octopus pairing offer` shows. */
export function PairServer() {
  const [address, setAddress] = createSignal("");
  const [code, setCode] = createSignal("");
  const complete = () => address().trim() !== "" && code().trim() !== "";
  const pair = (event: SubmitEvent) => {
    event.preventDefault();
    if (!complete()) return;
    backend.serverPair(address().trim(), code().trim()).then(() => showModal(null), report);
  };
  return (
    <form class="pane dialog" onSubmit={pair}>
      <TextField label="address" placeholder="mac-mini.local:7300" focus value={address()} change={setAddress} />
      <TextField label="code" placeholder="from octopus pairing offer" value={code()} change={setCode} />
      <Actions submit="Pair" complete={complete()} />
    </form>
  );
}
