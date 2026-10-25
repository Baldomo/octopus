import * as monaco from "monaco-editor/editor/editor.api";
import "monaco-editor/basic-languages/monaco.contribution";
import EditorWorker from "monaco-editor/editor/editor.worker?worker";
import { hex, px, styleOf, token } from "./tokens";

self.MonacoEnvironment = { getWorker: () => new EditorWorker() };

const THEME = "octopus";

monaco.editor.defineTheme(THEME, {
  base: "vs-dark",
  inherit: true,
  rules: [],
  colors: {
    "editor.background": hex(token("--term")),
    "editor.foreground": hex(styleOf(["code-ed"], "color")),
    "editorLineNumber.foreground": hex(styleOf(["code-ed", "ln"], "color")),
    "editorLineNumber.activeForeground": hex(token("--dim")),
    "editorCursor.foreground": hex(token("--text")),
    "editor.selectionBackground": hex(token("--line2")),
    "editor.lineHighlightBackground": hex(token("--term")),
    "editorWidget.background": hex(token("--raised")),
    "editorWidget.border": hex(token("--line2")),
    "scrollbarSlider.background": hex(token("--line2")),
  },
});

export function createEditor(host: HTMLElement) {
  return monaco.editor.create(host, {
    theme: THEME,
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
