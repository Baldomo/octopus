import { HUES, TONES, type Swatch } from "./generated";

/** Every swatch a section takes: the hues across, the tones down. */
export const SWATCHES: Swatch[] = TONES.flatMap((tone) => HUES.map((hue) => ({ hue, tone })));

export const sameSwatch = (a: Swatch, b: Swatch | null | undefined) => a.hue === b?.hue && a.tone === b?.tone;

/** What picking `swatch` picks: null, the auto color, for the `auto` swatch, else the swatch itself. */
export const pickSwatch = (swatch: Swatch, auto: Swatch | null | undefined) =>
  sameSwatch(swatch, auto) ? null : swatch;
