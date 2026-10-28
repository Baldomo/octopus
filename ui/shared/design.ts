import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import type { Plugin } from "vite";

const DESIGN = fileURLToPath(new URL("../../DESIGN.html", import.meta.url));
const MODULE = "virtual:design.css";
const RESOLVED = `\0${MODULE}.css`;

/** Serves DESIGN.html's token and component style blocks as one stylesheet. */
export function design(): Plugin {
  return {
    name: "octopus-design",
    resolveId: (id) => (id === MODULE ? RESOLVED : undefined),
    load(id) {
      if (id !== RESOLVED) return;
      this.addWatchFile(DESIGN);
      const html = readFileSync(DESIGN, "utf8");
      return ["tokens", "components"]
        .map((block) => {
          const found = html.match(new RegExp(`<style id="${block}">([\\s\\S]*?)</style>`));
          if (!found) throw new Error(`DESIGN.html has no <style id="${block}"> block`);
          return found[1];
        })
        .join("\n");
    },
  };
}
