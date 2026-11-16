# tidewatch

Tide times for Saltmarsh Cove, on one page.

The harbor master posts a week of readings in `data/harbor.csv`. `src/forecast.js` reads them, `src/tides.js` lists
today's highs and lows, and `src/chart.js` draws the week as a line.

## Run it

The page serves on port 4100.

## Test it

```sh
make test
```
