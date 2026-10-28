import type { Glyph as Kind } from "./generated";

export const statusWord: Record<Kind, string> = {
  working: "working",
  waiting: "needs you",
  idle: "idle",
  completed: "completed",
  cancelled: "cancelled",
};

const look: Record<Kind, { class: string; text: string }> = {
  working: { class: "spin", text: "" },
  waiting: { class: "dot needs", text: "" },
  idle: { class: "dot idle", text: "" },
  completed: { class: "ended", text: "✓" },
  cancelled: { class: "ended", text: "✕" },
};

export function Glyph(props: { glyph: Kind }) {
  return (
    <span class="glyph">
      <span class={look[props.glyph].class}>{look[props.glyph].text}</span>
    </span>
  );
}
