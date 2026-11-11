import { defineConfig } from "vite";
import solid from "vite-plugin-solid";
import { design } from "../shared/design.ts";
import { pdfAssets } from "../shared/pdfjs.ts";

export default defineConfig({
  plugins: [design(["DESIGN.html", ["tokens", "components"]]), pdfAssets(), solid()],
  clearScreen: false,
  build: { target: "es2022", chunkSizeWarningLimit: 4096 },
});
