import mono from "@fontsource-variable/jetbrains-mono/files/jetbrains-mono-latin-wght-normal.woff2?url";
import terminalBold from "./fonts/MesloLGSNF-Bold.woff2?url";
import terminalBoldItalic from "./fonts/MesloLGSNF-BoldItalic.woff2?url";
import terminalItalic from "./fonts/MesloLGSNF-Italic.woff2?url";
import terminalRegular from "./fonts/MesloLGSNF-Regular.woff2?url";

/**
 * The family names the design's `--ui`, `--mono` and terminal tokens ask for, served from the bundle, with the Inter
 * build the app asks for: the desktop's weights alone, or the phone's weights and optical sizes.
 */
const fonts = (inter: string) => [
  new FontFace("Inter", `url(${inter}) format("woff2")`, { weight: "100 900" }),
  new FontFace("JetBrains Mono", `url(${mono}) format("woff2")`, { weight: "100 800" }),
  new FontFace("MesloLGS NF", `url(${terminalRegular}) format("woff2")`),
  new FontFace("MesloLGS NF", `url(${terminalBold}) format("woff2")`, { weight: "bold" }),
  new FontFace("MesloLGS NF", `url(${terminalItalic}) format("woff2")`, { style: "italic" }),
  new FontFace("MesloLGS NF", `url(${terminalBoldItalic}) format("woff2")`, { weight: "bold", style: "italic" }),
];

export const loadFonts = (inter: string) =>
  Promise.all(fonts(inter).map(async (font) => document.fonts.add(await font.load())));
