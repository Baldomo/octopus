// Draws a week of tides as one line, highs and lows marked.

const WIDTH = 640;
const HEIGHT = 160;

export function weekChart(days) {
  const tides = days.flatMap((day) => day.tides);
  const step = WIDTH / (tides.length - 1);
  const y = (height) => HEIGHT - (height / 4.5) * HEIGHT;
  const points = tides.map((tide, at) => `${at * step},${y(tide.height)}`);
  const marks = tides.map(
    (tide, at) => `<circle cx="${at * step}" cy="${y(tide.height)}" r="3" class="${tide.kind}" />`,
  );
  return `<svg viewBox="0 0 ${WIDTH} ${HEIGHT}"><polyline points="${points.join(" ")}" />${marks.join("")}</svg>`;
}
