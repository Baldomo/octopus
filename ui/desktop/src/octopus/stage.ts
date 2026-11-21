/**
 * The first open's canvas: the show's octopus, drawn by its rig, at rest on the window, and the moves the beats ask of
 * it. Every place a move takes is in the canvas's CSS pixels; every move eases on the film's curves, lands where
 * `finish` puts it, and under reduced motion becomes a plain fade with the octopus holding still.
 */
import { clamp, inOut, lerp, out, smooth } from "./ease";
import { ACROSS, inks, rig, type ArmPose, type Look, type Pose } from "./rig";

export type Point = { x: number; y: number };
export type Rect = { x: number; y: number; w: number; h: number };
export type Motion = { finish: () => void; done: Promise<void> };

/** How much of the title bar's mark the octopus fills. */
const MARK_FILL = 0.94;
/** Seconds each move takes. */
const T = { lunge: 0.45, release: 0.55, recoil: 0.45, grab: 0.4, carry: 0.9, drop: 0.3, contract: 0.95, fade: 0.18 };
/**
 * How many times its own length an arm may stretch, how far in radians a gripping tip curls over what it holds, and
 * how much of that curl a pointing tip keeps.
 */
const STRETCH = 20;
const GRIP = 0.7;
const BECKON = 0.35;
/** Where a carried card starts, in rig units from the octopus's centre: beside its upper right. */
const CARRY_FROM = { x: 520, y: -470 };

/**
 * The arm that reaches toward `at` from the octopus at `from`: of the arms rooted in the skirt on that side, the one
 * whose tip points most sideways at rest, so a reach leaves the body among the other arms the way that arm already does.
 */
const reacher = (from: Point, at: Point) => {
  const left = at.x < from.x;
  return rig()
    .arms.filter((name) => {
      const [x, y] = rig().root(name);
      return y > 0 && x < 0 === left;
    })
    .map((name) => {
      const [x, y] = rig().tip({}, name);
      return { name, rise: Math.abs(y / x) };
    })
    .reduce((a, b) => (b.rise < a.rise ? b : a)).name;
};
/**
 * A reach keeps its arm's rest curve for its first `FROM`, turns onto its own course across the next `OVER`, and
 * turns `ARC` radians on that course, rising steeply and arriving level.
 */
const FROM = 0.25;
const OVER = 0.3;
const ARC = 1.2;
/** Radians the share of an arm beyond its unfurled one curls back by. */
const CURL_BACK = 2.8;
const turn = (a: number, b: number) => Math.atan2(Math.sin(a - b), Math.cos(a - b));

/** Where the octopus is: its centre, CSS pixels per rig unit, how much of its idle it moves, and how it shows. */
type Body = { at: Point; scale: number; live: number; alpha: number; tile: number };
/**
 * An arm out holding a point, in CSS pixels: how far it has unfurled toward it (0 curled at rest, 1 there), how much
 * its tip hooks round the point, how much it sways, and how much of it shows over the arm at rest, wholly unless it is
 * fading back.
 */
type Hold = { arm: string; at: Point; reach: number; hook: number; sway: number; seen?: number };
type Run = { start: number; duration: number; apply: (t: number) => void; resolve: () => void };

/** The point `e` of the way from `from` to `to`, arcing up by `lift` at the middle. */
const glide = (from: Point, to: Point, e: number, lift = 0): Point => ({
  x: lerp(from.x, to.x, e),
  y: lerp(from.y, to.y, e) - Math.sin(Math.PI * e) * lift,
});

/**
 * The pose that lays arm `name` toward `target` (rig units from the octopus's centre): out of the body along its rest
 * curve, then rising steeply and arriving level, `wave` swaying its middle, its tip hooked by `hook`, stretched so the
 * tip lands on the target once `reach` is 1. Short of 1, the share of the arm beyond the unfurled one curls back.
 */
function aim(name: string, target: Point, reach: number, hook: number, wave: number): ArmPose {
  const r = rig();
  const [rx, ry] = r.root(name);
  const straight = Math.atan2(target.y - ry, target.x - rx);
  const side = target.x < rx ? 1 : -1;
  const shape = (dir: number, stretch: number, front: number): ArmPose => ({
    stretch: stretch * front,
    from: FROM,
    over: OVER,
    heading: (u) => {
      const v = Math.max(0, (u - FROM) / (1 - FROM));
      return (
        dir +
        side * ARC * (0.5 - v) +
        wave * Math.sin(Math.PI * v) -
        side * hook * GRIP * smooth(0.8, 1, v) +
        (v > front ? -CURL_BACK * side * ((v - front) / (1 - front + 1e-3)) : 0)
      );
    },
  });
  const want = Math.hypot(target.x - rx, target.y - ry);
  let [dir, stretch] = [straight, 0];
  for (let i = 0; i < 6; i++) {
    const [tx, ty] = r.tip({ arms: { [name]: shape(dir, stretch, 1) } }, name);
    dir += turn(straight, Math.atan2(ty - ry, tx - rx));
    const got = Math.hypot(tx - rx, ty - ry);
    stretch = clamp((1 + stretch) * (want / got) - 1, -0.4, STRETCH - 1);
  }
  return shape(dir, stretch, clamp(reach));
}

export class Stage {
  private body: Body = { at: { x: 0, y: 0 }, scale: 0.3, live: 1, alpha: 1, tile: 0 };
  private hold: Hold | null = null;
  private runs = new Set<Run>();
  private context: CanvasRenderingContext2D;
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
    if (still) this.body.live = 0;
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

  /** Rests the octopus at `at`, `span` CSS pixels across. */
  place(at: Point, span: number) {
    Object.assign(this.body, { at, scale: span / ACROSS });
  }

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
    return this.tween(T.fade, (t) => apply(lerp(from, to, out(t / T.fade))));
  }

  /* ---------- the moves ---------- */

  /** The arm that points toward the window's edge at `at` unfurls and stretches across the window to hook it. */
  grow(at: Point): Motion {
    if (this.still) return this.tween(0, () => {});
    const arm = reacher(this.body.at, at);
    return this.tween(T.lunge, (t) => {
      const e = t / T.lunge;
      this.hold = { arm, at, reach: out(e), hook: smooth(0.7, 1, e), sway: 0 };
    });
  }

  /** The hooked edge slides from `from` to `to` over `duration` seconds on `--ease-pull`, the arm slinging it. */
  pull(from: Point, to: Point, duration: number): Motion {
    if (this.still) return this.tween(0, () => {});
    const { arm } = this.hold!;
    return this.tween(duration, (t) => {
      this.hold = { arm, at: glide(from, to, inOut(t / duration)), reach: 1, hook: 1, sway: 0 };
    });
  }

  /** The arm lets go of the edge at `at` and settles pointing at it, its tip `gap` CSS pixels clear of it. */
  release(at: Point, gap: number): Motion {
    const arm = this.hold?.arm ?? reacher(this.body.at, at);
    const from = this.hold?.at ?? at;
    const rest = { x: at.x + gap, y: at.y };
    const pose = (e: number, seen: number) => {
      this.hold = { arm, at: glide(from, rest, e), reach: 1, hook: 1 - (1 - BECKON) * e, sway: e, seen };
    };
    if (this.still) return this.fade((seen) => pose(1, seen), 0, 1);
    return this.tween(T.release, (t) => pose(out(t / T.release), 1));
  }

  /** The holding arm lets go and curls back into its rest. */
  recoil(): Motion {
    const held = this.hold;
    if (!held) return this.tween(0, () => {});
    if (this.still) return this.fade((seen) => (this.hold = seen > 0 ? { ...held, seen } : null), 1, 0);
    return this.tween(T.recoil, (t) => {
      const e = inOut(t / T.recoil);
      this.hold = e >= 1 ? null : { ...held, reach: 1 - e, hook: 0, sway: 1 - e };
    });
  }

  /**
   * An arm takes the card, `to`'s size, from beside the octopus's upper right by its top left corner and swings it to
   * `to`, then lets go; `show` places the card each frame with how much of it shows.
   */
  carry(to: Rect, show: (at: Rect, seen: number) => void): Motion {
    if (this.still) return this.tween(T.carry, (t) => show(to, Math.sin(Math.PI * (t / T.carry))));
    const { at, scale } = this.body;
    const from: Rect = { ...to, x: at.x + CARRY_FROM.x * scale, y: at.y + CARRY_FROM.y * scale };
    const arm = reacher(at, from);
    const total = T.grab + T.carry + T.recoil;
    return this.tween(total, (t) => {
      if (t < T.grab) {
        const e = t / T.grab;
        this.hold = { arm, at: from, reach: smooth(0, 1, e), hook: smooth(0.6, 1, e), sway: 0 };
        show(from, smooth(0.4, 1, e));
        return;
      }
      const e = inOut((t - T.grab) / T.carry);
      const card = { ...to, ...glide(from, to, e, 0.1 * Math.hypot(to.x - from.x, to.y - from.y)) };
      if (t < T.grab + T.carry) {
        this.hold = { arm, at: card, reach: 1, hook: 1, sway: 0 };
        show(card, 1);
        return;
      }
      const y = (t - T.grab - T.carry) / T.recoil;
      this.hold = y >= 1 ? null : { arm, at: to, reach: 1 - inOut(y), hook: 0, sway: 0 };
      show(to, 1 - smooth(0, T.drop / T.recoil, y));
    });
  }

  /**
   * The octopus contracts into the title bar's mark at `mark`, along one eased curve: it shrinks as it goes, stills,
   * and gathers its paper tile with corners `radius`.
   */
  contract(mark: Rect, radius: number): Motion {
    this.radius = radius / mark.w;
    this.hold = null;
    if (this.still) return this.fade((seen) => (this.body.alpha = seen), 1, 0);
    const [from, scale] = [{ ...this.body.at }, this.body.scale];
    const to = { x: mark.x + mark.w / 2, y: mark.y + mark.h / 2 };
    const bend = { x: from.x + (to.x - from.x) * 0.3, y: from.y + (to.y - from.y) * 0.85 };
    const end = (mark.w * MARK_FILL) / ACROSS;
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
    this.paint(now);
  }

  /** The pose at `t`: every arm swimming gently with the octopus's idle, and `held`'s arm laid toward its point. */
  private pose(t: number, held: Hold | null): Pose {
    const { at, scale, live } = this.body;
    const arms: Record<string, ArmPose> = Object.fromEntries(
      rig().arms.map((name) => [name, { wave: 0.09 * live, phase: t * 0.45 }]),
    );
    if (held) {
      const target = { x: (held.at.x - at.x) / scale, y: (held.at.y - at.y) / scale };
      arms[held.arm] = aim(held.arm, target, held.reach, held.hook, 0.12 * held.sway * live * Math.sin(t * 1.6));
    }
    return { arms, breathe: 0.015 * live * Math.sin(t * 1.9) };
  }

  private paint(t: number) {
    const { width, height, dpr } = this.size;
    const [context, body] = [this.context, this.body];
    const [x, y] = [body.at.x * dpr, body.at.y * dpr];
    context.setTransform(1, 0, 0, 1, 0, 0);
    context.clearRect(0, 0, width * dpr, height * dpr);
    context.globalAlpha = body.alpha * body.tile;
    if (body.tile > 0) tile(context, x, y, (ACROSS * body.scale * dpr) / MARK_FILL, this.radius);
    context.globalAlpha = body.alpha;
    const held = this.hold;
    const look = {
      pupil: held ? [Math.sign(held.at.x - body.at.x), Math.sign(held.at.y - body.at.y)] : [0, 0],
      lid: smooth(0, 0.06, 0.12 - Math.abs((t % 4.3) - 0.12)) * body.live,
    } satisfies Look;
    const k = body.scale * dpr;
    if (held && (held.seen ?? 1) < 1) {
      octopus(context, x, y, k, this.pose(t, null), look);
      context.globalAlpha = body.alpha * held.seen!;
      octopus(context, x, y, k, this.pose(t, held), { only: held.arm });
    } else octopus(context, x, y, k, this.pose(t, held), look);
    context.globalAlpha = 1;
  }
}

/** The paper tile, `side` device pixels across about (x, y), its corners `radius` of its side. */
function tile(context: CanvasRenderingContext2D, x: number, y: number, side: number, radius: number) {
  context.fillStyle = inks().paper;
  context.beginPath();
  context.roundRect(x - side / 2, y - side / 2, side, side, side * radius);
  context.fill();
}

/** The octopus centred on (x, y), `k` device pixels to its unit. */
function octopus(context: CanvasRenderingContext2D, x: number, y: number, k: number, pose?: Pose, look?: Look) {
  context.setTransform(k, 0, 0, k, x, y);
  rig().draw(context, pose, look);
  context.setTransform(1, 0, 0, 1, 0, 0);
}

/** Paints the title bar's mark: the octopus at rest on its paper tile, filling the canvas. */
export function paintMark(canvas: HTMLCanvasElement) {
  const side = canvas.clientWidth * (devicePixelRatio || 1);
  [canvas.width, canvas.height] = [Math.round(side), Math.round(side)];
  const context = canvas.getContext("2d")!;
  const radius = parseFloat(getComputedStyle(canvas).borderTopLeftRadius) || 0;
  tile(context, side / 2, side / 2, side, radius / canvas.clientWidth);
  octopus(context, side / 2, side / 2, (side * MARK_FILL) / ACROSS);
}
