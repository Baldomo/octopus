import { defineConfig } from "vite";
import solid from "vite-plugin-solid";
import { design } from "../shared/design.ts";

export default defineConfig({
  plugins: [design(["DESIGN.html", ["tokens", "components"]]), solid()],
  clearScreen: false,
  build: { target: "es2022", chunkSizeWarningLimit: 4096 },
});
