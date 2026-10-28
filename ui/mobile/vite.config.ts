import { defineConfig } from "vite";
import solid from "vite-plugin-solid";
import { design } from "../shared/design.ts";

export default defineConfig({
  plugins: [design(["DESIGN.html", ["tokens"]], ["DESIGN-MOBILE.html", ["tokens", "components"]]), solid()],
  clearScreen: false,
  // light-dark() stays as written, so the appearance the client menu sets through `color-scheme` reaches every token.
  build: { target: "es2022", cssTarget: ["safari17.5", "chrome123"], chunkSizeWarningLimit: 4096 },
});
