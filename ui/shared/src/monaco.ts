import * as monaco from "monaco-editor/editor/editor.api";
import "monaco-editor/basic-languages/monaco.contribution";
import EditorWorker from "monaco-editor/editor/editor.worker?worker";
import { createEffect, createRoot } from "solid-js";
import { colors, px, type Scheme, scheme, styleOf, token } from "./tokens";

self.MonacoEnvironment = { getWorker: () => new EditorWorker() };

const CHROME = {
  "editor.background": "--code",
  "editor.foreground": "--term-text",
  "editorLineNumber.foreground": "--gutter",
  "editorLineNumber.activeForeground": "--dim",
  "editorCursor.foreground": "--text",
  "editor.selectionBackground": "--line2",
  "editor.inactiveSelectionBackground": "--line2",
  "editor.selectionHighlightBackground": "--accent-ink",
  "editor.findMatchBackground": "--accent-line",
  "editor.findMatchHighlightBackground": "--accent-ink",
  "editor.lineHighlightBackground": "--code",
  "editorIndentGuide.background1": "--code-guide",
  "editorIndentGuide.activeBackground1": "--code-guide-active",
  "editorBracketHighlight.foreground1": "--syn-bracket-1",
  "editorBracketHighlight.foreground2": "--syn-bracket-2",
  "editorBracketHighlight.foreground3": "--syn-bracket-3",
  "editorWidget.background": "--raised",
  "editorWidget.border": "--line2",
  "scrollbarSlider.background": "--line2",
};

/** The role each Monaco token takes; a token matches its longest listed prefix. */
const SYNTAX = {
  keyword: "--syn-keyword",
  constant: "--syn-keyword",
  tag: "--syn-keyword",
  "keyword.flow": "--syn-flow",
  string: "--syn-string",
  "attribute.value": "--syn-string",
  comment: "--syn-comment",
  number: "--syn-number",
  "attribute.value.number": "--syn-number",
  "attribute.value.unit": "--syn-number",
  type: "--syn-type",
  variable: "--syn-variable",
  key: "--syn-key",
  "attribute.name": "--syn-key",
  "variable.parameter": "--syn-key",
  regexp: "--syn-regexp",
  annotation: "--syn-annotation",
  metatag: "--syn-annotation",
  invalid: "--syn-invalid",
  delimiter: "--syn-delimiter",
};

function theme(current: Scheme): monaco.editor.IStandaloneThemeData {
  const color = colors(current);
  return {
    base: current === "dark" ? "vs-dark" : "vs",
    inherit: false,
    rules: [
      { token: "emphasis", fontStyle: "italic" },
      { token: "strong", fontStyle: "bold" },
      ...Object.entries(SYNTAX).map(([prefix, role]) => ({ token: prefix, foreground: color(role).slice(1, 7) })),
    ],
    colors: Object.fromEntries(Object.entries(CHROME).map(([key, name]) => [key, color(name)])),
  };
}

const THEMES = { light: "octopus-light", dark: "octopus-dark" } as const;
for (const each of ["light", "dark"] as const) monaco.editor.defineTheme(THEMES[each], theme(each));
createRoot(() => createEffect(() => monaco.editor.setTheme(THEMES[scheme()])));

export function createEditor(host: HTMLElement) {
  return monaco.editor.create(host, {
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
