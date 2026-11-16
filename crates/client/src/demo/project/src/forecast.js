// Reads the harbor master's CSV into days of tides, each tide at its local time.

export function parseForecast(text) {
  const days = new Map();
  for (const row of text.trim().split("\n").slice(1)) {
    if (row.trim() === "") continue;
    const [day, time, height, kind] = row.split(",");
    const [clock, offset] = [time.slice(0, 5), time.slice(5) || "+00:00"];
    const tide = { clock, offset, height: Number(height), kind };
    if (!days.has(day)) days.set(day, []);
    days.get(day).push(tide);
  }
  return [...days].map(([day, tides]) => ({ day, tides }));
}
