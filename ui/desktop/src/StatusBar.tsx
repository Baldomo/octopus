import { Show } from "solid-js";
import { openView, serverLabel, serverView, ui } from "@octopus/shared/state.ts";

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
                {serverLabel(open().at.server)} · {server()?.reachable ? `${server()?.rtt_ms ?? "–"}ms` : "unreachable"}
              </span>
              <span>{open().panels.length} pty</span>
              <Show when={open().task.branch}>{(branch) => <span>⎇ {branch()}</span>}</Show>
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
