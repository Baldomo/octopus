import * as monaco from "monaco-editor/editor/editor.api";
import "monaco-editor/basic-languages/monaco.contribution";
import EditorWorker from "monaco-editor/editor/editor.worker?worker";
import { createEffect, createRoot, on } from "solid-js";
import { colors, px, scheme, styleOf, token } from "./tokens";

self.MonacoEnvironment = { getWorker: () => new EditorWorker() };

const THEMES = { light: "octopus-light", dark: "octopus-dark" } as const;
for (const each of ["light", "dark"] as const) {
  const color = colors(each);
  monaco.editor.defineTheme(THEMES[each], {
    base: each === "dark" ? "vs-dark" : "vs",
    inherit: true,
    rules: [],
    colors: {
      "editor.background": color("--term"),
      "editor.foreground": color("--term-text"),
      "editorLineNumber.foreground": color("--gutter"),
      "editorLineNumber.activeForeground": color("--dim"),
      "editorCursor.foreground": color("--text"),
      "editor.selectionBackground": color("--line2"),
      "editor.lineHighlightBackground": color("--term"),
      "editorWidget.background": color("--raised"),
      "editorWidget.border": color("--line2"),
      "scrollbarSlider.background": color("--line2"),
    },
  });
}
createRoot(() => createEffect(on(scheme, (current) => monaco.editor.setTheme(THEMES[current]), { defer: true })));

export function createEditor(host: HTMLElement) {
  return monaco.editor.create(host, {
    theme: THEMES[scheme()],
    model: null,
    fontFamily: token("--mono"),
    fontSize: px("--fs-mono"),
    lineHeight: Number.parseFloat(styleOf(["code-ed"], "lineHeight")),
    minimap: { enabled: false },
    automaticLayout: true,
    scrollBeyondLastLine: false,
    renderLineHighlight: "none",
    lineNumbersMinChars: 3,
    padding: { top: Number.parseFloat(styleOf(["code-ed", "src"], "paddingTop")) },
  });
}

export { monaco };
