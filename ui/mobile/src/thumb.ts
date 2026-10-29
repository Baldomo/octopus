import { onCleanup } from "solid-js";

/**
 * Draws a segmented control's or a tab bar's selection as one thumb under its `.on` item, which slides to the item
 * chosen next and resizes to it, instead of the item drawing its own.
 */
export function slidingThumb(bar: HTMLElement) {
  bar.classList.add("thumbed");
  const thumb = document.createElement("i");
  thumb.className = "thumb-on";
  bar.prepend(thumb);
  const place = () => {
    if (!thumb.isConnected) bar.prepend(thumb);
    const on = bar.querySelector<HTMLElement>(":scope > .on");
    thumb.style.opacity = on ? "1" : "0";
    if (!on) return;
    Object.assign(thumb.style, {
      translate: `${on.offsetLeft}px ${on.offsetTop}px`,
      width: `${on.offsetWidth}px`,
      height: `${on.offsetHeight}px`,
    });
  };
  const changes = new MutationObserver(place);
  changes.observe(bar, { subtree: true, childList: true, attributes: true, attributeFilter: ["class"] });
  const sizes = new ResizeObserver(place);
  sizes.observe(bar);
  onCleanup(() => {
    changes.disconnect();
    sizes.disconnect();
  });
  requestAnimationFrame(() => {
    place();
    requestAnimationFrame(() => thumb.classList.add("placed"));
  });
}
