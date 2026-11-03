import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import type { Plugin } from "vite";

const MODULE = "virtual:design.css";
const RESOLVED = `\0${MODULE}.css`;

/** A design file at the repo root, and the ids of the style blocks taken from it, in order. */
type Blocks = [file: string, ids: string[]];

/** Serves the named style blocks of the repo's design files as one stylesheet, later blocks winning. */
export function design(...sources: Blocks[]): Plugin {
  return {
    name: "octopus-design",
    resolveId: (id) => (id === MODULE ? RESOLVED : undefined),
    load(id) {
      if (id !== RESOLVED) return;
      return sources
        .flatMap(([file, ids]) => {
          const path = fileURLToPath(new URL(`../../${file}`, import.meta.url));
          this.addWatchFile(path);
          const html = readFileSync(path, "utf8");
          return ids.map((block) => {
            const found = html.match(new RegExp(`<style id="${block}">([\\s\\S]*?)</style>`));
            if (!found) throw new Error(`${file} has no <style id="${block}"> block`);
            return found[1];
          });
        })
        .join("\n");
    },
  };
}
