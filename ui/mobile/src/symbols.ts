import symbols from "@fontsource/noto-sans-symbols/files/noto-sans-symbols-symbols-400-normal.woff2?url";
import symbols2 from "@fontsource/noto-sans-symbols-2/files/noto-sans-symbols-2-symbols-400-normal.woff2?url";

/**
 * Meslo lacks some glyphs agents print, such as ⏺, ⏵ and ⎿, and the OS would draw them from its color emoji font, so
 * two flat symbol fonts come right after it and draw them in the terminal's color.
 */
export const TERMINAL_FONT = `"MesloLGS NF", "Noto Sans Symbols 2", "Noto Sans Symbols", monospace`;

export const loadSymbols = () =>
  Promise.all(
    [
      new FontFace("Noto Sans Symbols 2", `url(${symbols2}) format("woff2")`),
      new FontFace("Noto Sans Symbols", `url(${symbols}) format("woff2")`),
    ].map(async (font) => document.fonts.add(await font.load())),
  );
