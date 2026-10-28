import inter from "@fontsource-variable/inter/files/inter-latin-wght-normal.woff2?url";
import mono from "@fontsource-variable/jetbrains-mono/files/jetbrains-mono-latin-wght-normal.woff2?url";
import terminalBold from "./fonts/MesloLGSNF-Bold.ttf?url";
import terminalBoldItalic from "./fonts/MesloLGSNF-BoldItalic.ttf?url";
import terminalItalic from "./fonts/MesloLGSNF-Italic.ttf?url";
import terminalRegular from "./fonts/MesloLGSNF-Regular.ttf?url";

/** The family names DESIGN.html's `--ui`, `--mono` and `--mono-terminal` tokens ask for, served from the bundle. */
const fonts = [
  new FontFace("Inter", `url(${inter}) format("woff2")`, { weight: "100 900" }),
  new FontFace("JetBrains Mono", `url(${mono}) format("woff2")`, { weight: "100 800" }),
  new FontFace("MesloLGS NF", `url(${terminalRegular}) format("truetype")`),
  new FontFace("MesloLGS NF", `url(${terminalBold}) format("truetype")`, { weight: "bold" }),
  new FontFace("MesloLGS NF", `url(${terminalItalic}) format("truetype")`, { style: "italic" }),
  new FontFace("MesloLGS NF", `url(${terminalBoldItalic}) format("truetype")`, { weight: "bold", style: "italic" }),
];

export const loadFonts = () => Promise.all(fonts.map(async (font) => document.fonts.add(await font.load())));
