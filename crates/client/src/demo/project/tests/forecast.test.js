import { assert, test } from "./harness.js";
import { parseForecast } from "../src/forecast.js";

const csv = "day,time,height_m,kind\nmon,04:12+01:00,4.1,high\nmon,10:31+01:00,0.9,low\n\ntue,05:01,4.0,high\n";

test("groups tides by day", () => {
  assert.deepEqual(
    parseForecast(csv).map((day) => day.day),
    ["mon", "tue"],
  );
});

test("keeps each tide's offset", () => {
  assert.equal(parseForecast(csv)[0].tides[0].offset, "+01:00");
});

test("reads a bare time as +00:00", () => {
  assert.equal(parseForecast(csv)[1].tides[0].offset, "+00:00");
});

test("skips blank rows", () => {
  assert.equal(parseForecast(csv).flatMap((day) => day.tides).length, 3);
});
