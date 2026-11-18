/**
 * The first open's canvas: the film's octopus at rest on the window, its tentacles, and the moves the beats ask of it.
 * Every place a move takes is in the canvas's CSS pixels; every move eases on the film's curves, lands where `finish`
 * puts it, and under reduced motion becomes a plain fade with the octopus holding still.
 */
import {
  body,
  INKS,
  poseAt,
  REACH,
  WHITE,
  type ArmName,
  type Cell,
  type Emit,
  type Ink,
  type Point,
  type Rect,
} from "./body";
import { clamp, inOut, out, smooth } from "./ease";
import { fills, reseed, type Fills } from "./print";
import {
  armOf,
  cellsOf,
  corner,
  edge,
  reach,
  reaching,
  recoiling,
  releasing,
  type Hold,
  type Moment,
  type Strand,
} from "./tentacle";

export type { Point, Rect };
export type Motion = { finish: () => void; done: Promise<void> };

/** How much of the title bar's mark the octopus fills, and the mark's corner over its side. */
const MARK_FILL = 0.94;
/**
 * Seconds each move takes; the pull trails the sidebar by `lag`, so the hook stays behind its edge. `cover` is the share
 * of a grow, and of a recoil's end, over which a tentacle's skin takes over from its arm's logo squares: as long as the
 * tentacle lies within a few units of the arm's curl.
 */
const T = {
  cover: 0.05,
  lag: 0.04,
  print: 0.42,
  grow: 0.9,
  release: 0.7,
  recoil: 0.55,
  carry: 0.9,
  drop: 0.3,
  contract: 0.95,
  fade: 0.18,
};
/** Squares draw up to a device pixel larger than their side, so the sway's small differences never open a seam. */
const grow = (side: number) => clamp(side / 4, 0, 1);
/** The order inks paint in, the outline last. */
const ORDER = [3, 2, 4, 5, WHITE, 0];

const color = (name: string) => getComputedStyle(document.documentElement).getPropertyValue(name).trim();
const inkColors = () => INKS.map((ink) => color(`--octo-${ink}`));

/** Where the octopus is: its centre, CSS pixels per world unit, how much of its idle it moves, and how it shows. */
type Body = { at: Point; scale: number; live: number; alpha: number; tile: number; print: number };
/** A tentacle out of its arm: its strand, its pose, how it shows, and how far its tip sways at rest. */
/**
 * A tentacle out of its arm: its strand, its pose, how it shows, how far its tip sways at rest, and how much its skin
 * has taken over from the arm's logo squares beneath it, from 0 (the logo's arm) to 1.
 */
type Tentacle = { strand: Strand; moment: Moment; alpha: number; sway: number; skin: number };
type Run = { start: number; duration: number; apply: (t: number) => void; resolve: () => void };

/** Paints squares in world units onto the canvas, grouped by ink. */
class Squares {
  private bins: number[][] = INKS.map(() => []);
  add(x: number, y: number, size: number, ink: Ink) {
    this.bins[ink]!.push(x, y, size);
  }
  paint(context: CanvasRenderingContext2D, place: Body, dpr: number, all: (CanvasPattern | string)[], solid: string[]) {
    const [k, ox, oy] = [place.scale * dpr, place.at.x * dpr, place.at.y * dpr];
    for (const ink of ORDER) {
      const bin = this.bins[ink]!;
      if (!bin.length) continue;
      context.beginPath();
      for (let i = 0; i < bin.length; i += 3) {
        const side = bin[i + 2]! * k + grow(bin[i + 2]! * k);
        context.rect(ox + bin[i]! * k - side / 2, oy + bin[i + 1]! * k - side / 2, side, side);
      }
      const alpha = context.globalAlpha;
      if (place.print < 1 || typeof all[ink] === "string") {
        context.fillStyle = solid[ink]!;
        context.fill();
      }
      if (place.print > 0 && typeof all[ink] !== "string") {
        context.globalAlpha = alpha * place.print;
        context.fillStyle = all[ink]!;
        context.fill();
        context.globalAlpha = alpha;
      }
      bin.length = 0;
    }
  }
}

export class Stage {
  readonly body: Body = { at: { x: 0, y: 0 }, scale: 0.3, live: 1, alpha: 1, tile: 0, print: 0 };
  private tentacles = new Map<ArmName, Tentacle>();
  private runs = new Set<Run>();
  private context: CanvasRenderingContext2D;
  private solid = inkColors();
  private all: Fills = this.solid;
  private squares = new Squares();
  private size = { width: 0, height: 0, dpr: 1 };
  /** Seconds the stage has played: a frame the window was too busy to paint advances it by one frame, never more. */
  private clock = 0;
  private last = performance.now();
  private frame = 0;
  /** The mark's corner radius over its side, which the gathered tile takes. */
  private radius = 0;

  constructor(
    private canvas: HTMLCanvasElement,
    private still: boolean,
  ) {
    this.context = canvas.getContext("2d")!;
    void fills(this.context, this.solid).then((loaded) => {
      this.all = loaded;
      this.tween(T.print, (t) => (this.body.print = out(t / T.print)));
    });
    const tick = () => {
      this.tick();
      this.frame = requestAnimationFrame(tick);
    };
    this.frame = requestAnimationFrame(tick);
  }

  stop() {
    cancelAnimationFrame(this.frame);
    for (const run of this.runs) run.resolve();
    this.runs.clear();
  }

  /** Fits the canvas to the window, at its device pixel ratio. */
  resize(width: number, height: number) {
    const dpr = devicePixelRatio || 1;
    this.size = { width, height, dpr };
    [this.canvas.width, this.canvas.height] = [Math.round(width * dpr), Math.round(height * dpr)];
    [this.canvas.style.width, this.canvas.style.height] = [`${width}px`, `${height}px`];
  }

  /** Puts the octopus at `at`, `span` CSS pixels across. */
  place(at: Point, span: number) {
    Object.assign(this.body, { at, scale: span / (2 * REACH) });
  }

  private world = (p: Point): Point => ({
    x: (p.x - this.body.at.x) / this.body.scale,
    y: (p.y - this.body.at.y) / this.body.scale,
  });

  /** Calls `apply` every frame with the seconds since the move began, and last with `duration`. */
  tween(duration: number, apply: (t: number) => void): Motion {
    let resolve!: () => void;
    const done = new Promise<void>((settle) => (resolve = settle));
    const run: Run = { start: this.clock, duration, apply, resolve };
    this.runs.add(run);
    apply(0);
    return {
      finish: () => {
        if (!this.runs.delete(run)) return;
        apply(duration);
        resolve();
      },
      done,
    };
  }

  private fade(apply: (seen: number) => void, from: number, to: number) {
    return this.tween(T.fade, (t) => apply(from + (to - from) * out(t / T.fade)));
  }

  /* ---------- the moves ---------- */

  /** The hook a tentacle holds the sidebar's edge by at `at`, arriving from the right. */
  private hook = (at: Point): Hold => edge(this.world(at), Math.PI, -1);

  /** One tentacle grows from the lower left arm to hook the edge at `at`, its tip leading along a natural curve. */
  grow(at: Point): Motion {
    if (this.still) return this.tween(0, () => {});
    const strand = reach(armOf("leftBottom"), this.hook(at), -1, 1);
    return this.tween(T.grow, (t) =>
      this.tentacles.set("leftBottom", {
        strand,
        moment: reaching(strand, t / T.grow),
        alpha: 1,
        sway: 0,
        skin: smooth(0, T.cover, t / T.grow),
      }),
    );
  }

  /** The hooked edge slides from `from` to `to` over `duration` seconds on `--ease-pull`, the tentacle drawing it. */
  pull(from: Point, to: Point, duration: number): Motion {
    if (this.still) return this.tween(0, () => {});
    return this.tween(duration + T.lag, (t) => {
      const e = inOut((t - T.lag) / duration);
      const at = { x: from.x + (to.x - from.x) * e, y: from.y + (to.y - from.y) * e };
      const strand = reach(armOf("leftBottom"), this.hook(at), -1, 1);
      this.tentacles.set("leftBottom", {
        strand,
        moment: reaching(strand, 1 + t / T.grow),
        alpha: 1,
        sway: 0,
        skin: 1,
      });
    });
  }

  /** The tentacle lets go of the edge at `at` and settles with its tip `gap` CSS pixels clear of it. */
  release(at: Point, gap: number): Motion {
    const strand = reach(armOf("leftBottom"), this.hook(at), -1, 1);
    const back = 17 * 2.3 + gap / this.body.scale;
    const pose = (y: number, alpha: number) =>
      this.tentacles.set("leftBottom", {
        strand,
        moment: releasing(strand, back, y),
        alpha,
        sway: smooth(0.6, 1, y),
        skin: 1,
      });
    if (this.still) return this.fade((seen) => pose(1, seen), 0, 1);
    return this.tween(T.release, (t) => pose(t / T.release, 1));
  }

  /** The tentacle of arm `name` lets go and shortens back into its arm. */
  recoil(name: ArmName): Motion {
    const was = this.tentacles.get(name);
    if (was === undefined) return this.tween(0, () => {});
    const end = (t: number, last: number) => t >= last && this.tentacles.delete(name);
    if (this.still) return this.tween(T.fade, (t) => ((was.alpha = 1 - out(t / T.fade)), end(t, T.fade)));
    const from = { ...was.moment };
    return this.tween(T.recoil, (t) => {
      const y = t / T.recoil;
      this.tentacles.set(name, {
        ...was,
        moment: recoiling(was.strand, from, y),
        sway: 0,
        skin: 1 - smooth(1 - T.cover, 1, y),
      });
      end(t, T.recoil);
    });
  }

  /**
   * The upper right arm takes the card at `from` by its top left corner and carries it to `to`, then lets go; `show`
   * places the card each frame with how much of it shows.
   */
  carry(from: Rect, to: Rect, radius: number, show: (at: Rect, seen: number) => void): Motion {
    const lerp = (e: number): Rect => ({
      x: from.x + (to.x - from.x) * e,
      y: from.y + (to.y - from.y) * e - Math.sin(Math.PI * e) * 0.12 * Math.hypot(to.x - from.x, to.y - from.y),
      w: from.w,
      h: from.h,
    });
    if (this.still) return this.tween(T.carry, (t) => show(to, Math.sin(Math.PI * (t / T.carry))));
    const hold = (card: Rect) => {
      const p = this.world(card);
      const s = this.body.scale;
      return reach(
        armOf("upperRight"),
        corner({ x: p.x, y: p.y, w: card.w / s, h: card.h / s }, radius / s, "topLeft", 1),
        1,
        2,
      );
    };
    const [grow, carry] = [T.grow * 0.6, T.carry];
    let last: Moment | null = null;
    const total = grow + carry + T.recoil;
    return this.tween(total, (t) => {
      if (t < grow + carry) {
        const card = lerp(inOut((t - grow) / carry));
        const strand = hold(card);
        last = reaching(strand, t / grow);
        this.tentacles.set("upperRight", {
          strand,
          moment: last,
          alpha: 1,
          sway: 0,
          skin: smooth(0, T.cover, t / grow),
        });
        show(card, smooth(0.45, 0.9, t / grow));
        return;
      }
      const y = (t - grow - carry) / T.recoil;
      const strand = hold(to);
      last ??= reaching(strand, 2);
      if (y >= 1) this.tentacles.delete("upperRight");
      else
        this.tentacles.set("upperRight", {
          strand,
          moment: recoiling(strand, last, y),
          alpha: 1,
          sway: 0,
          skin: 1 - smooth(1 - T.cover, 1, y),
        });
      show(to, 1 - smooth(0, T.drop / T.recoil, y));
    });
  }

  /**
   * The octopus contracts into the title bar's mark at `mark`, along one eased curve: it shrinks as it goes, stills,
   * and gathers its paper tile with corners `radius`.
   */
  contract(mark: Rect, radius: number): Motion {
    this.radius = radius / mark.w;
    if (this.still) return this.fade((seen) => (this.body.alpha = seen), 1, 0);
    const [from, scale] = [{ ...this.body.at }, this.body.scale];
    const to = { x: mark.x + mark.w / 2, y: mark.y + mark.h / 2 };
    const bend = { x: from.x + (to.x - from.x) * 0.3, y: from.y + (to.y - from.y) * 0.85 };
    const end = (mark.w * MARK_FILL) / (2 * REACH);
    return this.tween(T.contract, (t) => {
      const e = inOut(t / T.contract);
      const u = 1 - e;
      Object.assign(this.body, {
        at: {
          x: u * u * from.x + 2 * u * e * bend.x + e * e * to.x,
          y: u * u * from.y + 2 * u * e * bend.y + e * e * to.y,
        },
        scale: scale * (end / scale) ** e,
        live: 1 - smooth(0, 0.8, e),
        tile: smooth(0.4, 1, e),
        print: 1 - smooth(0.15, 0.75, e),
      });
    });
  }

  /** Whether a move is under way. */
  moving = () => this.runs.size > 0;

  /* ---------- a frame ---------- */

  private tick() {
    const at = performance.now();
    this.clock += Math.min((at - this.last) / 1000, 1 / 30);
    this.last = at;
    const now = this.clock;
    for (const run of [...this.runs]) {
      const t = Math.min(now - run.start, run.duration);
      run.apply(t);
      if (t >= run.duration) (this.runs.delete(run), run.resolve());
    }
    this.paint(this.still ? 0 : now);
  }

  private paint(t: number) {
    const { width, height, dpr } = this.size;
    const [context, place] = [this.context, this.body];
    context.setTransform(1, 0, 0, 1, 0, 0);
    context.clearRect(0, 0, width * dpr, height * dpr);
    reseed(this.all, t);
    context.globalAlpha = place.alpha;
    if (place.tile > 0) {
      const side = (2 * REACH * place.scale * dpr) / MARK_FILL;
      context.globalAlpha = place.alpha * place.tile;
      context.fillStyle = color("--octo-paper");
      context.beginPath();
      context.roundRect(place.at.x * dpr - side / 2, place.at.y * dpr - side / 2, side, side, side * this.radius);
      context.fill();
      context.globalAlpha = place.alpha;
    }
    const live = this.still ? 0 : place.live;
    const add: Emit = (x, y, size, ink) => this.squares.add(x, y, size, ink);
    const covered = (cell: Cell) => [...this.tentacles.values()].some(({ strand }) => strand.arm.hidden.has(cell));
    body(t, live, (cell) => !covered(cell), add);
    this.squares.paint(context, place, dpr, this.all, this.solid);
    for (const { strand, moment, alpha, sway, skin } of this.tentacles.values()) {
      if (skin < 1) {
        context.globalAlpha = place.alpha * alpha * (1 - smooth(1 - T.cover, 1, skin));
        body(t, live, (cell) => strand.arm.hidden.has(cell), add);
        this.squares.paint(context, place, dpr, this.all, this.solid);
      }
      context.globalAlpha = place.alpha * alpha * skin;
      const pose = sway
        ? { ...moment, open: moment.open + sway * live * 0.1 * Math.sin((2 * Math.PI * t) / 4.4) }
        : moment;
      const end = strand.path.len - strand.holding - 30;
      const start = Math.max(60, end - 450);
      cellsOf(strand, pose, (x, y, size, ink, s) => {
        const share = 1 - smooth(start, end, s);
        const p = share > 0 ? poseAt(x, y, t, live) : { x, y };
        this.squares.add(x + (p.x - x) * share, y + (p.y - y) * share, size, ink);
      });
      this.squares.paint(context, place, dpr, this.all, this.solid);
    }
    context.globalAlpha = 1;
  }
}

/** Paints the title bar's mark: the octopus at rest on its paper tile, filling the canvas. */
export function paintMark(canvas: HTMLCanvasElement) {
  const side = canvas.clientWidth;
  const dpr = devicePixelRatio || 1;
  [canvas.width, canvas.height] = [Math.round(side * dpr), Math.round(side * dpr)];
  const context = canvas.getContext("2d")!;
  const radius = parseFloat(getComputedStyle(canvas).borderTopLeftRadius) || 0;
  context.fillStyle = color("--octo-paper");
  context.beginPath();
  context.roundRect(0, 0, side * dpr, side * dpr, radius * dpr);
  context.fill();
  const solid = inkColors();
  const place: Body = {
    at: { x: side / 2, y: side / 2 },
    scale: (side * MARK_FILL) / (2 * REACH),
    live: 0,
    alpha: 1,
    tile: 1,
    print: 0,
  };
  const squares = new Squares();
  body(
    0,
    0,
    () => true,
    (x, y, size, ink) => squares.add(x, y, size, ink),
  );
  squares.paint(context, place, dpr, solid, solid);
}
