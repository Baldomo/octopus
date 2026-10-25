import { Show } from "solid-js";
import { openView, serverView, ui } from "./state";

export function StatusBar() {
  return (
    <footer class="status">
      <Show when={openView()}>
        {(open) => {
          const server = () => serverView(open().at.server);
          return (
            <>
              <span>
                <Show when={server()?.reachable} fallback={<span>○</span>}>
                  <span class="live">●</span>
                </Show>{" "}
                {open().at.server} · {server()?.reachable ? `${server()?.rtt_ms ?? "–"}ms` : "unreachable"}
              </span>
              <span>{open().task.panels.length} pty</span>
              <span>⎇ {open().task.branch ?? "root checkout"}</span>
            </>
          );
        }}
      </Show>
      <span class="spacer" />
      <Show when={ui.problem}>{(text) => <span class="problem">{text()}</span>}</Show>
      <Show when={openView()}>{(open) => <span>harness {open().task.harness}</span>}</Show>
    </footer>
  );
}
