import { createSignal, onCleanup } from "solid-js";

/** A time of day in the locale's clock, or on the 24-hour one a design asks for. */
export const clock = (at: number, hourCycle?: "h23") =>
  new Date(at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", hourCycle });

export function ago(at: number, now: number) {
  const minutes = Math.floor((now - at) / 60_000);
  if (minutes < 1) return "just now";
  if (minutes < 60) return `${minutes} min ago`;
  const hours = Math.floor(minutes / 60);
  return hours < 24 ? `${hours} h ago` : `${Math.floor(hours / 24)} d ago`;
}

export function ticking() {
  const [now, setNow] = createSignal(Date.now());
  const timer = setInterval(() => setNow(Date.now()), 1000);
  onCleanup(() => clearInterval(timer));
  return now;
}
