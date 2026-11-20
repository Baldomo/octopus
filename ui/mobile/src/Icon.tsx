import { For } from "solid-js";

/** The design's icons, each drawn in currentColor at the stroke `.ic` sets. */
const SYMBOLS = {
  back: ["0 0 20 20", `<path d="M12.5 4.5 7 10l5.5 5.5"/>`],
  fwd: ["0 0 20 20", `<path d="M7.5 4.5 13 10l-5.5 5.5"/>`],
  more: [
    "0 0 20 20",
    `<circle cx="4.5" cy="10" r="1.1" fill="currentColor" stroke="none"/><circle cx="10" cy="10" r="1.1" fill="currentColor" stroke="none"/><circle cx="15.5" cy="10" r="1.1" fill="currentColor" stroke="none"/>`,
  ],
  search: ["0 0 20 20", `<circle cx="9" cy="9" r="5.5"/><path d="m13 13 3.5 3.5"/>`],
  gear: [
    "0 0 24 24",
    `<path stroke-width="2.1" d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.730l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z"/><circle stroke-width="2.1" cx="12" cy="12" r="3"/>`,
  ],
  minus: ["0 0 16 16", `<path d="M3.5 8h9"/>`],
  key: ["0 0 16 16", `<circle cx="5.5" cy="10.5" r="3"/><path d="m7.6 8.4 5.4-5.4M11 5l1.5 1.5M9.5 6.5 11 8"/>`],
  archive: [
    "0 0 16 16",
    `<rect x="2.5" y="3" width="11" height="3" rx="1"/><path d="M3.5 6v6.5a1 1 0 0 0 1 1h7a1 1 0 0 0 1-1V6M6.5 8.5h3"/>`,
  ],
  plus: ["0 0 20 20", `<path d="M10 4v12M4 10h12"/>`],
  car: ["0 0 16 16", `<path d="m4 6 4 4 4-4"/>`],
  up: ["0 0 16 16", `<path d="m4 10 4-4 4 4"/>`],
  x: ["0 0 16 16", `<path d="m5 5 6 6M11 5l-6 6"/>`],
  branch: [
    "0 0 16 16",
    `<circle cx="4.5" cy="3.5" r="1.5"/><circle cx="4.5" cy="12.5" r="1.5"/><circle cx="11.5" cy="5" r="1.5"/><path d="M4.5 5v6M11.5 6.5c0 3-7 2-7 4.5"/>`,
  ],
  pen: ["0 0 16 16", `<path d="M10.5 3.5 12.5 5.5 6 12H4v-2z"/>`],
  kbd: [
    "0 0 16 16",
    `<rect x="2.5" y="4.5" width="11" height="7.5" rx="1.5"/><path d="M5 6.75h.01M7 6.75h.01M9 6.75h.01M11 6.75h.01M5.5 9.5h5"/>`,
  ],
  check: ["0 0 16 16", `<path d="m3.5 8.5 3 3 6-7"/>`],
  close: ["0 0 20 20", `<path d="m5.5 5.5 9 9M14.5 5.5l-9 9"/>`],
  folder: [
    "0 0 16 16",
    `<path d="M2.5 4.5a1 1 0 0 1 1-1h3l1.5 1.5h4.5a1 1 0 0 1 1 1v5.5a1 1 0 0 1-1 1h-9a1 1 0 0 1-1-1z"/>`,
  ],
  file: [
    "0 0 16 16",
    `<path d="M4.5 2.5h4.5l3 3v7a1 1 0 0 1-1 1h-6.5a1 1 0 0 1-1-1v-9a1 1 0 0 1 1-1z"/><path d="M9 2.5v3h3"/>`,
  ],
  globe: [
    "0 0 16 16",
    `<circle cx="8" cy="8" r="5.5"/><path d="M2.5 8h11M8 2.5c1.7 1.8 1.7 9.2 0 11M8 2.5c-1.7 1.8-1.7 9.2 0 11"/>`,
  ],
  qr: [
    "0 0 16 16",
    `<rect x="2.5" y="2.5" width="4" height="4" rx=".5"/><rect x="9.5" y="2.5" width="4" height="4" rx=".5"/><rect x="2.5" y="9.5" width="4" height="4" rx=".5"/><path d="M9.5 9.5h1.5v1.5M13.5 9.5v1M9.5 13.5h4M12 11.5v2"/>`,
  ],
  board: [
    "0 0 16 16",
    `<rect x="2.5" y="3" width="3" height="10" rx=".8"/><rect x="6.5" y="3" width="3" height="7" rx=".8"/><rect x="10.5" y="3" width="3" height="5" rx=".8"/>`,
  ],
  copy: [
    "0 0 16 16",
    `<rect x="5.5" y="5.5" width="8" height="8" rx="1.5"/><path d="M10.5 3.5v-.5a.5.5 0 0 0-.5-.5h-6.5a1 1 0 0 0-1 1v6.5a.5.5 0 0 0 .5.5h.5"/>`,
  ],
  share: ["0 0 16 16", `<path d="M8 2.5v8M5 5.5l3-3 3 3M4.5 8.5h-1v5h9v-5h-1"/>`],
  mic: ["0 0 16 16", `<rect x="6" y="2" width="4" height="7.5" rx="2"/><path d="M3.5 7.5a4.5 4.5 0 0 0 9 0M8 12v2"/>`],
  micoff: [
    "0 0 16 16",
    `<path d="M10 6.5V4a2 2 0 0 0-4 0v3.5a2 2 0 0 0 3.2 1.6M3.5 7.5a4.5 4.5 0 0 0 7 3.7M12.5 7.5c0 .6-.1 1.1-.3 1.6M8 12v2M2.5 2.5l11 11"/>`,
  ],
  reload: ["0 0 20 20", `<path d="M15.5 10a5.5 5.5 0 1 1-1.6-3.9M14.5 3.5v3h-3"/>`],
  chev: ["0 0 16 16", `<path d="m6 4 4 4-4 4"/>`],
  warn: ["0 0 16 16", `<circle cx="8" cy="8" r="5.5"/><path d="M8 5v3.5M8 11h.01"/>`],
  lock: ["0 0 16 16", `<rect x="3.5" y="7" width="9" height="6.5" rx="1.5"/><path d="M5.5 7V5a2.5 2.5 0 0 1 5 0v2"/>`],
  sun: ["0 0 16 16", `<circle cx="8" cy="8" r="5.5"/><path d="M8 2.5v11a5.5 5.5 0 0 0 0-11z" fill="currentColor"/>`],
  zoom: ["0 0 16 16", `<circle cx="7" cy="7" r="4.5"/><path d="m10.5 10.5 3 3M5 7h4M7 5v4"/>`],
  info: ["0 0 16 16", `<circle cx="8" cy="8" r="5.5"/><path d="M8 7.5V11M8 5h.01"/>`],
  eye: [
    "0 0 20 20",
    `<path d="M2.5 10s2.7-5 7.5-5 7.5 5 7.5 5-2.7 5-7.5 5-7.5-5-7.5-5z"/><circle cx="10" cy="10" r="2.2"/>`,
  ],
  code: ["0 0 20 20", `<path d="m7 6-4 4 4 4M13 6l4 4-4 4"/>`],
  paste: ["0 0 16 16", `<rect x="3.5" y="3" width="9" height="10.5" rx="1.5"/><path d="M6 3V2.5h4V3M6 7h4M6 10h3"/>`],
  attach: [
    "0 0 16 16",
    `<path d="M10 5.6 6.4 9.2a1.12 1.12 0 0 0 1.6 1.6L12 6.8a2.24 2.24 0 0 0-3.2-3.2L4.8 7.6a3.36 3.36 0 0 0 4.8 4.8l2.8-2.8"/>`,
  ],
  cam: [
    "0 0 16 16",
    `<path d="M2.5 5.5a1 1 0 0 1 1-1h1.5l1-1.5h4l1 1.5h1.5a1 1 0 0 1 1 1v6a1 1 0 0 1-1 1h-9a1 1 0 0 1-1-1z"/><circle cx="8" cy="8.5" r="2.2"/>`,
  ],
} as const;

export type IconName = keyof typeof SYMBOLS;

export const Sprite = () => (
  <svg width="0" height="0" style={{ position: "absolute" }}>
    <For each={Object.entries(SYMBOLS)}>
      {([name, [viewBox, body]]) => <symbol id={`i-${name}`} viewBox={viewBox} innerHTML={body} />}
    </For>
  </svg>
);

/** A 16pt icon, or a 20pt one in a round button. */
export const Icon = (props: { name: IconName; big?: boolean; class?: string }) => (
  <svg class={props.class ? `ic ${props.class}` : "ic"} classList={{ s: !props.big }}>
    <use href={`#i-${props.name}`} />
  </svg>
);
