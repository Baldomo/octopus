import { createSignal } from "solid-js";
import { backend } from "./backend";
import { Actions, TextField } from "./Dialog";
import { report, showModal } from "./state";

/** Pairs with a server on another machine through the invite its `octopus pairing offer` shows. */
export function PairServer() {
  const [invite, setInvite] = createSignal("");
  const complete = () => invite().trim() !== "";
  const pair = (event: SubmitEvent) => {
    event.preventDefault();
    if (!complete()) return;
    backend.serverPair(invite().trim()).then(() => showModal(null), report);
  };
  return (
    <form class="pane dialog" onSubmit={pair}>
      <TextField label="invite" placeholder="from octopus pairing offer" focus value={invite()} change={setInvite} />
      <Actions submit="Pair" complete={complete()} />
    </form>
  );
}
