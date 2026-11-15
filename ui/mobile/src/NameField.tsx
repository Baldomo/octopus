import { createSignal, Show } from "solid-js";
import { said } from "@octopus/shared/errors.ts";
import { feel } from "./feel";
import { Icon, type IconName } from "./Icon";
import { Mi, Note } from "./parts";

/**
 * Edits a name in place in a focused field: return applies a changed, non-empty name, and leaving the field keeps the
 * old one. A refusal shows under the field, which stays for another try.
 */
export function NameField(props: { value: string; apply: (name: string) => Promise<unknown>; done: () => void }) {
  const [problem, setProblem] = createSignal<string | null>(null);
  let applying = false;
  return (
    <>
      <label class="field">
        <input
          class="name"
          spellcheck={false}
          autocapitalize="off"
          autocorrect="off"
          enterkeyhint="done"
          value={props.value}
          ref={(input) =>
            requestAnimationFrame(() => {
              input.focus();
              input.select();
            })
          }
          onKeyDown={(event) => {
            if (event.key !== "Enter") return;
            event.preventDefault();
            const name = event.currentTarget.value.trim();
            if (name === "" || name === props.value) return props.done();
            applying = true;
            props.apply(name).then(
              () => {
                feel("success");
                props.done();
              },
              (error) => {
                feel("error");
                applying = false;
                setProblem(said(error));
              },
            );
          }}
          onBlur={() => !applying && problem() === null && props.done()}
        />
      </label>
      <Show when={problem()}>{(shown) => <Note err>{shown()}</Note>}</Show>
    </>
  );
}

export function NameItem(props: {
  icon: IconName;
  label: string;
  value: string;
  apply: (name: string) => Promise<unknown>;
}) {
  const [naming, setNaming] = createSignal(false);
  return (
    <Show
      when={naming()}
      fallback={
        <Mi slot={<Icon name={props.icon} />} tap={() => setNaming(true)}>
          {props.label}
        </Mi>
      }
    >
      <NameField value={props.value} apply={props.apply} done={() => setNaming(false)} />
    </Show>
  );
}
