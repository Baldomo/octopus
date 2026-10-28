import { report } from "@octopus/shared/state.ts";

/** Edits a name in place: return applies a changed, non-empty name, and leaving the field keeps the old one. */
export function Rename(props: { value: string; apply: (name: string) => Promise<unknown>; done: () => void }) {
  return (
    <input
      class="rename"
      spellcheck={false}
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
        props.done();
        if (name !== "" && name !== props.value) props.apply(name).catch(report);
      }}
      onBlur={() => props.done()}
    />
  );
}
