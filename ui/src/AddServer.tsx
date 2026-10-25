import { createSignal } from "solid-js";
import { backend } from "./backend";
import { Actions, TextField } from "./Dialog";
import { report, setOverlay } from "./state";

export function AddServer() {
  const [name, setName] = createSignal("");
  const [socket, setSocket] = createSignal("");
  const complete = () => name().trim() !== "" && socket().trim() !== "";
  const add = (event: SubmitEvent) => {
    event.preventDefault();
    if (!complete()) return;
    backend.serverAdd(name().trim(), socket().trim()).then(() => setOverlay(null), report);
  };
  return (
    <form class="pane dialog" onSubmit={add}>
      <TextField label="name" placeholder="mac-mini" focus value={name()} change={setName} />
      <TextField label="socket" placeholder="/path/to/server.sock" value={socket()} change={setSocket} />
      <Actions submit="Add" complete={complete()} />
    </form>
  );
}
