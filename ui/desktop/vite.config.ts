import { defineConfig } from "vite";
import solid from "vite-plugin-solid";
import { pdfAssets } from "../shared/pdfjs.ts";

export default defineConfig({
  plugins: [pdfAssets(), solid()],
  clearScreen: false,
  build: {
    sourcemap: false,
    target: "es2022",
    // light-dark() stays as written, so a token resolves against whichever scheme the element asking for it carries.
    cssTarget: ["safari17.5", "chrome123"],
    chunkSizeWarningLimit: 4096,
    // show.html is the first open's show, which plays in its own window over every display.
    rollupOptions: { input: { main: "index.html", show: "show.html" } },
  },
});
