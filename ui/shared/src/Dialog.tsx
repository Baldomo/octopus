import { For, type JSX } from "solid-js";
import { showModal } from "./state";

export const autofocus = (input: HTMLInputElement) => queueMicrotask(() => input.focus());

export function Field(props: { label: string; children: JSX.Element }) {
  return (
    <>
      <div class="gh">{props.label}</div>
      {props.children}
    </>
  );
}

export function TextField(props: {
  label: string;
  value: string;
  placeholder?: string;
  focus?: boolean;
  change: (value: string) => void;
}) {
  return (
    <Field label={props.label}>
      <div class="search">
        <input
          ref={(input) => props.focus && autofocus(input)}
          spellcheck={false}
          placeholder={props.placeholder}
          value={props.value}
          onInput={(event) => props.change(event.currentTarget.value)}
        />
      </div>
    </Field>
  );
}

export function Segments<T extends string>(props: {
  options: readonly T[];
  value: T;
  choose: (value: T) => void;
  label?: (value: T) => string;
  off?: (value: T) => boolean;
}) {
  return (
    <div class="seg">
      <For each={props.options}>
        {(option) => (
          <button
            type="button"
            classList={{ on: props.value === option }}
            disabled={props.off?.(option)}
            onClick={() => props.choose(option)}
          >
            {props.label?.(option) ?? option}
          </button>
        )}
      </For>
    </div>
  );
}

export function Actions(props: { submit: string; complete: boolean }) {
  return (
    <div class="actions">
      <button type="button" class="btn" onClick={() => showModal(null)}>
        Cancel
      </button>
      <button type="submit" class="btn live" disabled={!props.complete}>
        {props.submit}
      </button>
    </div>
  );
}
