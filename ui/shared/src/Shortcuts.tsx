import { For } from "solid-js";
import { commands, hint, type Command } from "./commands";

const keyed = Object.values(commands).filter((command: Command) => command.key !== null);

export function Shortcuts() {
  return (
    <div class="pane dialog keys">
      <div class="title">
        Keyboard shortcuts<kbd>esc</kbd>
      </div>
      <div class="results">
        <table class="keytable">
          <For each={keyed}>
            {(command) => (
              <tr>
                <td>{command.label}</td>
                <td>
                  <kbd>{hint(command)}</kbd>
                </td>
              </tr>
            )}
          </For>
        </table>
      </div>
    </div>
  );
}
