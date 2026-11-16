import { defineConfig } from "vite";
import solid from "vite-plugin-solid";
import { pdfAssets } from "../shared/pdfjs.ts";

export default defineConfig({
  plugins: [pdfAssets(), solid()],
  clearScreen: false,
  // light-dark() stays as written, so a token resolves against whichever scheme the element asking for it carries.
  build: { target: "es2022", cssTarget: ["safari17.5", "chrome123"], chunkSizeWarningLimit: 4096 },
});
