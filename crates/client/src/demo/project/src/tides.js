import { parseForecast } from "./forecast.js";
import { weekChart } from "./chart.js";

const today = (days) => days[0];

export async function showTides(page) {
  const days = parseForecast(await (await fetch("../data/harbor.csv")).text());
  const rows = today(days).tides.map(
    (tide) => `<tr class="${tide.kind}"><td>${tide.clock}</td><td>${tide.kind}</td><td>${tide.height} m</td></tr>`,
  );
  page.querySelector("#today").innerHTML = rows.join("");
  page.querySelector("#week").innerHTML = weekChart(days);
}
