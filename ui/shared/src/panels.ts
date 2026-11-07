import { createMemo, For, type JSX } from "solid-js";
import type { PanelView } from "./generated";

/** A child unmounting with its panel gone from the list still reads the panel as it last was. */
export function EachPanel(props: { panels: PanelView[]; children: (panel: () => PanelView) => JSX.Element }) {
  return For({
    get each() {
      return props.panels.map((panel) => panel.id);
    },
    children: (id) => {
      const panel = createMemo<PanelView>((last) => props.panels.find((each) => each.id === id) ?? last!);
      return props.children(panel);
    },
  });
}
