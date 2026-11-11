import { readdirSync, readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import type { Plugin } from "vite";
import { PDF_ASSETS } from "./src/opening.ts";

/** The pdfjs-dist directories the PDF reader fetches from as a document needs them. */
const DIRECTORIES = ["cmaps", "standard_fonts", "wasm", "iccs"];
/** The script sandbox, which a reader that runs no document scripts never loads. */
const SCRIPTING = /^quickjs/;

/** Ships those directories with the build, under `PDF_ASSETS`. */
export function pdfAssets(): Plugin {
  const root = dirname(createRequire(import.meta.url).resolve("pdfjs-dist/package.json"));
  return {
    name: "octopus-pdf-assets",
    apply: "build",
    generateBundle() {
      for (const directory of DIRECTORIES)
        for (const name of readdirSync(join(root, directory)).filter((name) => !SCRIPTING.test(name)))
          this.emitFile({
            type: "asset",
            fileName: `${PDF_ASSETS}/${directory}/${name}`,
            source: readFileSync(join(root, directory, name)),
          });
    },
  };
}
