import { createMemo, For, type JSX } from "solid-js";
import type { Panel } from "./generated";

/**
 * One child per panel, kept by the panel's id while its fields change. A child unmounting with its panel gone from the
 * list still reads the panel as it last was.
 */
export function EachPanel(props: { panels: Panel[]; children: (panel: () => Panel) => JSX.Element }) {
  return For({
    get each() {
      return props.panels.map((panel) => panel.id);
    },
    children: (id) => {
      const panel = createMemo<Panel>((last) => props.panels.find((each) => each.id === id) ?? last!);
      return props.children(panel);
    },
  });
}
