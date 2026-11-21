// While the music is still trapped behind the screen the Mac seems to break, the way failing graphics hardware does:
// the cursor turns into the spinning beachball (freeze), a pixel row smears down the screen (smear), blocks land in
// the wrong place or decode to garbage colour (blocks), rows jitter sideways (buzz), the picture loses vertical sync
// and rolls (roll), the display drops out and comes back with every window gone (blackout), and everything at once
// (storm). Each glitch shows what lies beneath the windows, the wallpaper.

import { hash } from "../octopus/ease";
import { BLACK, C, cut, DPR, H, lx, ly, mouse, W, type Context } from "./display";
import { INFECTION } from "./music";
import { bare } from "./stills";

type Glitch = (g: Context, u: number, q: number, t: number) => void;
const LOOKS: Record<string, Glitch> = {
  freeze: beachball,
  smear,
  blocks,
  buzz,
  roll,
  blackout: blackOut,
  storm: (g, u, q) => {
    smear(g, u, q);
    blocks(g, u, q + 7);
    buzz(g, 1, q + 3);
  },
};

/** Draws every glitch under way at t. */
export function infect(g: Context, t: number) {
  for (const e of INFECTION) {
    const u = (t - e.t) / e.lasts;
    if (u >= 0 && u < 1) LOOKS[e.look]!(g, u, Math.floor(t * 30) * 13 + e.t * 1000, t);
  }
}

function beachball(g: Context, _u: number, _q: number, t: number) {
  const x = lx(mouse.x);
  const y = ly(mouse.y);
  const r = 11 * DPR;
  const spin = g.createConicGradient((t / 0.7) * Math.PI * 2, x, y);
  const hues = ["#f94144", "#f8961e", "#f9c74f", "#90be6d", "#43aa8b", "#277da1", "#9b5de5", "#f94144"];
  hues.forEach((c, i) => spin.addColorStop(i / (hues.length - 1), c));
  g.beginPath();
  g.arc(x, y, r + DPR, 0, Math.PI * 2);
  g.fillStyle = "rgba(0,0,0,0.35)";
  g.fill();
  g.beginPath();
  g.arc(x, y, r, 0, Math.PI * 2);
  g.fillStyle = spin;
  g.fill();
  const shine = g.createRadialGradient(x - r * 0.35, y - r * 0.4, 0, x, y, r);
  shine.addColorStop(0, "rgba(255,255,255,0.7)");
  shine.addColorStop(0.5, "rgba(255,255,255,0.1)");
  shine.addColorStop(1, "rgba(0,0,0,0.2)");
  g.fillStyle = shine;
  g.fill();
}

function smear(g: Context, u: number, q: number) {
  for (let b = 0; b < 1 + Math.floor(u * 3); b++) {
    const y = hash(q + b) * H * 0.9;
    const h = H * (0.05 + 0.3 * hash(q + b * 3)) * Math.min(1, u * 4 + 0.3);
    cut(g, bare, 0, y, W, 1, 0, y, W, h);
  }
}

function blocks(g: Context, u: number, q: number) {
  const s = 16 * DPR;
  for (let k = 0; k < 6 + u * 20; k++) {
    const x = Math.floor((hash(q + k * 3) * W) / s) * s;
    const y = Math.floor((hash(q + k * 5) * H) / s) * s;
    const w = s * (1 + Math.floor(hash(q + k * 7) * 12));
    const h = s * (1 + Math.floor(hash(q + k * 11) * 3));
    if (hash(q + k * 13) > 0.3) {
      const fromX = x + Math.round((hash(q + k * 17) - 0.5) * 8) * s;
      const fromY = y + Math.round((hash(q + k * 19) - 0.5) * 4) * s;
      cut(g, bare, fromX, fromY, w, h, x, y, w, h);
    } else {
      g.fillStyle = hash(q + k * 23) > 0.5 ? "#00e06a" : "#e000c8";
      g.globalAlpha = 0.85;
      g.fillRect(x, y, w, h);
      g.globalAlpha = 1;
    }
  }
}

function buzz(g: Context, u: number, q: number) {
  for (let b = 0; b < 2 + u * 10; b++) {
    const y = hash(q + b * 3) * H;
    const h = DPR * (1 + hash(q + b * 5) * 6 * (0.3 + u));
    const dx = (hash(q + b * 7) - 0.5) * DPR * 40 * (0.3 + u);
    cut(g, bare, 0, y, W, h, dx, y, W, h);
  }
}

function roll(g: Context, u: number) {
  const off = (u < 0.8 ? u / 0.8 : 0) * H * 0.6;
  g.fillStyle = BLACK;
  g.fillRect(0, 0, W, H);
  g.drawImage(bare, 0, off);
  g.drawImage(bare, 0, off - H - 18 * DPR);
}

function blackOut(g: Context, u: number) {
  g.fillStyle = u < 0.6 ? BLACK : C.white;
  g.fillRect(0, 0, W, H);
}
