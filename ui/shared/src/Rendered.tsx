import DOMPurify from "dompurify";
import { marked } from "marked";
import { createEffect, createSignal, Match, onCleanup, onMount, Switch } from "solid-js";
import { backend } from "./backend";
import { report } from "./state";
import { extensionOf, renderingOf, type TaskFiles } from "./taskFiles";

const MIME: Record<string, string> = {
  css: "text/css",
  png: "image/png",
  jpg: "image/jpeg",
  jpeg: "image/jpeg",
  gif: "image/gif",
  webp: "image/webp",
  avif: "image/avif",
  svg: "image/svg+xml",
  ico: "image/x-icon",
};

function checkoutPath(from: string, reference: string) {
  if (reference === "" || /^([a-z][a-z\d+.-]*:|\/\/|#)/i.test(reference)) return null;
  const parts = reference.startsWith("/") ? [] : from.split("/").slice(0, -1);
  const [location = ""] = reference.split(/[?#]/);
  try {
    for (const part of location.split("/")) {
      if (part === "..") parts.pop();
      else if (part !== "." && part !== "") parts.push(decodeURIComponent(part));
    }
  } catch {
    return null;
  }
  return parts.join("/");
}

/** A missing file renders empty and is reported once. */
class Assets {
  private readonly texts = new Map<string, Promise<string>>();
  private readonly dataUrls = new Map<string, Promise<string>>();

  constructor(private readonly files: TaskFiles) {}

  private once(cache: Map<string, Promise<string>>, path: string, read: (blob: Blob) => Promise<string>) {
    let found = cache.get(path);
    if (found === undefined) {
      const type = MIME[extensionOf(path)] ?? "application/octet-stream";
      found = backend
        .fileBytes(this.files.at, path)
        .then((bytes) => read(new Blob([bytes], { type })))
        .catch((error) => {
          report(`${path}: ${error}`);
          return "";
        });
      cache.set(path, found);
    }
    return found;
  }

  text(path: string) {
    return this.once(this.texts, path, (blob) => blob.text());
  }

  dataUrl(path: string) {
    return this.once(
      this.dataUrls,
      path,
      (blob) =>
        new Promise((resolve, reject) => {
          const reader = new FileReader();
          reader.onload = () => resolve(String(reader.result));
          reader.onerror = () => reject(reader.error);
          reader.readAsDataURL(blob);
        }),
    );
  }

  inlineImages(root: ParentNode, from: string) {
    return Promise.all(
      IMAGES.flatMap(([selector, attribute]) =>
        [...root.querySelectorAll(selector)].map(async (element) => {
          const path = checkoutPath(from, element.getAttribute(attribute) ?? "");
          if (path !== null) element.setAttribute(attribute, await this.dataUrl(path));
        }),
      ),
    );
  }
}

const IMAGES: [string, string][] = [
  ["img[src]", "src"],
  ["source[src]", "src"],
  ["video[poster]", "poster"],
  ["image[href]", "href"],
];

async function markdown(text: string, from: string, assets: Assets) {
  const body = document.createElement("div");
  body.innerHTML = DOMPurify.sanitize(marked.parse(text, { async: false }), {
    FORBID_TAGS: ["style", "form"],
    FORBID_ATTR: ["style"],
  });
  await assets.inlineImages(body, from);
  return body.innerHTML;
}

async function page(text: string, from: string, assets: Assets) {
  const parsed = new DOMParser().parseFromString(text, "text/html");
  const inertLinks = parsed.createElement("base");
  inertLinks.target = "_blank";
  parsed.head.prepend(inertLinks);
  await Promise.all([
    ...[...parsed.querySelectorAll<HTMLLinkElement>("link[rel~=stylesheet][href]")].map(async (link) => {
      const path = checkoutPath(from, link.getAttribute("href") ?? "");
      if (path === null) return;
      const style = parsed.createElement("style");
      style.textContent = await assets.text(path);
      link.replaceWith(style);
    }),
    assets.inlineImages(parsed, from),
  ]);
  return `<!doctype html>${parsed.documentElement.outerHTML}`;
}

export function Rendered(props: { files: TaskFiles; path: string; handBack: () => void }) {
  let view!: HTMLDivElement;
  onMount(() => Boolean(document.activeElement?.closest(".files")) && view.focus());
  onCleanup(() => view.contains(document.activeElement) && props.handBack());
  const assets = new Assets(props.files);
  const model = props.files.buffer(props.path)?.model;
  const [text, setText] = createSignal(model?.getValue() ?? "");
  const listening = model?.onDidChangeContent(() => setText(model.getValue()));
  onCleanup(() => listening?.dispose());

  const rendering = renderingOf(props.path);
  const [html, setHtml] = createSignal("");
  createEffect(() => {
    const current = text();
    const render = rendering === "markdown" ? markdown : rendering === "page" ? page : undefined;
    render?.(current, props.path, assets).then((done) => text() === current && setHtml(done), report);
  });

  return (
    <div class="rendered over-editor" ref={view} tabIndex={-1}>
      <Switch>
        <Match when={rendering === "image"}>
          <div class="rendered-image">
            <img alt="" src={`data:image/svg+xml,${encodeURIComponent(text())}`} />
          </div>
        </Match>
        <Match when={rendering === "markdown"}>
          <div class="rendered-doc" innerHTML={html()} onClick={(event) => event.preventDefault()} />
        </Match>
        <Match when={rendering === "page"}>
          <iframe class="rendered-page" sandbox="" srcdoc={html()} title={props.path} onFocus={() => view.focus()} />
        </Match>
      </Switch>
    </div>
  );
}
