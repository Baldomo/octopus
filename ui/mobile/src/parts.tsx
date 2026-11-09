import { For, Show, type JSX } from "solid-js";
import type { Glyph as Status, ServerView } from "@octopus/shared/generated.ts";
import { turning } from "@octopus/shared/Glyph.tsx";
import { sectionView } from "@octopus/shared/state.ts";
import { swatchColor } from "@octopus/shared/tokens.ts";
import { feel } from "./feel";
import { Icon, type IconName } from "./Icon";
import { onPainted } from "./motion";
import { back } from "./nav";
import { slidingThumb } from "./thumb";

/** The color of the section named `name`, faint while the model lists none by that name. */
export const sectionColor = (name: string | undefined) => {
  const section = sectionView(name);
  return section === undefined ? "var(--faint)" : swatchColor(section.swatch);
};

/** A 20pt slot holding a glyph, caret or icon, centered on its text's line box. */
export const Slot = (props: { children?: JSX.Element; class?: string }) => (
  <span class={props.class ? `slot ${props.class}` : "slot"}>{props.children}</span>
);

export const Circle = (props: { icon: IconName; label: string; on?: boolean; tap: () => void }) => (
  <button class="circle" classList={{ on: props.on }} aria-label={props.label} onClick={() => props.tap()}>
    <Icon name={props.icon} big />
  </button>
);

const look: Record<Status, () => JSX.Element> = {
  working: () => (
    <svg class="spin" viewBox="0 0 12 12">
      <circle cx="6" cy="6" r="5" pathLength="100" ref={turning} />
    </svg>
  ),
  waiting: () => <i class="dot needs" />,
  idle: () => <i class="dot idle" />,
  completed: () => (
    <span class="end">
      <Icon name="check" />
    </span>
  ),
  cancelled: () => (
    <span class="end">
      <Icon name="x" />
    </span>
  ),
};

/** A task's status: working spins, waiting is amber, idle a ring, and an ended task its check or cross. */
export const Glyph = (props: { status: Status }) => <>{look[props.status]()}</>;

/** A server's dot: filled while reachable, hollow while not, an amber ring while the phone reconnects to it. */
export const ServerDot = (props: { server: ServerView | undefined; reconnecting?: boolean }) => (
  <i class="dot srv" classList={{ off: !props.server?.reachable && !props.reconnecting, re: props.reconnecting }} />
);

export const Id = (props: { children: JSX.Element; class?: string }) => (
  <span class={props.class ? `id ${props.class}` : "id"}>{props.children}</span>
);

/** A path cut from its start when it runs past the right rail, so its tail stays. */
export const Path = (props: { children: string }) => (
  <span class="path">
    <bdi>{props.children}</bdi>
  </span>
);

export const Why = (props: { children: JSX.Element }) => <span class="why">{props.children}</span>;

export const Chev = () => (
  <span class="chev">
    <Icon name="chev" />
  </span>
);

/** A 48pt menu item: its icon or glyph in the slot, its label, and its value, reason or chevron on the right rail. */
export function Mi(props: {
  slot?: JSX.Element;
  children: JSX.Element;
  end?: JSX.Element;
  class?: string;
  off?: boolean;
  danger?: boolean;
  tap?: () => void;
}) {
  return (
    <div
      class={props.class ? `mi ${props.class}` : "mi"}
      classList={{ off: props.off, danger: props.danger }}
      role="button"
      aria-disabled={props.off ? "true" : undefined}
      onClick={() => !props.off && props.tap?.()}
    >
      <Slot>{props.slot}</Slot>
      <span class="name">{props.children}</span>
      {props.end}
    </div>
  );
}

export const Sep = () => <div class="sep" />;
export const Gl = (props: { children: JSX.Element }) => <div class="gl">{props.children}</div>;
export const Flab = (props: { children: JSX.Element }) => <div class="flab">{props.children}</div>;
export const Para = (props: { children: JSX.Element; center?: boolean }) => (
  <div class="para" classList={{ c: props.center }}>
    {props.children}
  </div>
);
export const Note = (props: { children: JSX.Element; err?: boolean }) => (
  <div class="note" classList={{ err: props.err }}>
    <Show when={props.err}>
      <Slot>
        <Icon name="warn" />
      </Slot>
    </Show>
    <span class="name">{props.children}</span>
  </div>
);
export const Gap = (props: { n: 2 | 3 | 4 | 5 }) => <div class={`sp${props.n}`} />;

/** A segmented control, the current segment on the surface under a thumb that slides to each choice. */
export function Seg<T extends string>(props: {
  options: readonly (readonly [T, string])[];
  value: T;
  choose: (value: T) => void;
  wide?: boolean;
}) {
  return (
    <div class="seg" classList={{ wide: props.wide }} ref={slidingThumb}>
      <For each={props.options}>
        {([value, label]) => (
          <span
            role="button"
            classList={{ on: props.value === value }}
            onClick={() => {
              if (props.value !== value) feel("select");
              props.choose(value);
            }}
          >
            {label}
          </span>
        )}
      </For>
    </div>
  );
}

/** A 48pt filled field typing into its input, which the stylesheet rings while it has the focus. */
export function Field(props: {
  value: string;
  input: (value: string) => void;
  placeholder?: string;
  mono?: boolean;
  focus?: boolean;
  enter?: () => void;
}) {
  let input!: HTMLInputElement;
  onPainted(() => props.focus && input.focus({ preventScroll: true }));
  return (
    <label class="field" classList={{ mono: props.mono }}>
      <input
        class="name"
        value={props.value}
        placeholder={props.placeholder}
        spellcheck={false}
        autocapitalize="off"
        autocorrect="off"
        autocomplete="off"
        enterkeyhint="done"
        ref={input}
        onInput={(event) => props.input(event.currentTarget.value)}
        onKeyDown={(event) => {
          if (event.key !== "Enter") return;
          event.preventDefault();
          event.currentTarget.blur();
          props.enter?.();
        }}
      />
    </label>
  );
}

export const Primary = (props: { children: JSX.Element; tap: () => void; off?: boolean }) => (
  <button class="primary" disabled={props.off} onClick={() => props.tap()}>
    {props.children}
  </button>
);

export const Secondary = (props: { children: JSX.Element; tap: () => void }) => (
  <button class="secondary" onClick={() => props.tap()}>
    {props.children}
  </button>
);

export const Trail = (props: { parts: string[] }) => (
  <For each={props.parts}>
    {(part, nth) => (
      <>
        <Show when={nth() > 0}>
          <span class="sep">/</span>
        </Show>
        {part}
      </>
    )}
  </For>
);

/** A screen's top bar over a crumb in its section's color, back to the screen named `up`. */
export function CrumbBar(props: {
  up: string;
  color: string;
  crumb: JSX.Element;
  title: JSX.Element;
  end?: JSX.Element;
  slim?: boolean;
}) {
  return (
    <div class="bar" classList={{ slim: props.slim }}>
      <Circle icon="back" label={props.up} tap={back} />
      <div class="ttl" style={{ "--ws": props.color }}>
        <span class="crumb">
          <i />
          {props.crumb}
        </span>
        <b class="name">{props.title}</b>
      </div>
      {props.end}
    </div>
  );
}

/** A screen's 56pt top bar: back when the screen has one, then its title or crumbs over the title, then actions. */
export function Bar(props: { back?: () => void; children: JSX.Element; end?: JSX.Element }) {
  return (
    <div class="bar">
      <Show when={props.back}>{(back) => <Circle icon="back" label="Back" tap={back()} />}</Show>
      {props.children}
      {props.end}
    </div>
  );
}
