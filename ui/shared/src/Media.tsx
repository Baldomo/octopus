import workerUrl from "pdfjs-dist/legacy/build/pdf.worker.min.mjs?url";
import { createSignal, For, Match, onCleanup, onMount, Show, Switch } from "solid-js";
import { fetched, PDF_ASSETS, PDF_CHUNK, rangeOf, sizeOf } from "./opening";
import type { MediaView, TaskFiles } from "./taskFiles";

/** Loads the PDF a range at a time and draws its pages alone, with no text, form or script layer. Each page draws
 * near the view and drops far from it, so a long PDF holds a few pages' pixels. */
function Pdf(props: { url: string; scroller: () => HTMLElement }) {
  const [pages, setPages] = createSignal<number[]>([]);
  const [ratio, setRatio] = createSignal(Math.SQRT2);
  /** Each canvas drawn or being drawn, with what stops its drawing. */
  const drawing = new Map<HTMLCanvasElement, { cancelled: boolean; cancel?: () => void }>();
  /** Ends every read of the file when the view goes, and marks it gone. */
  const reads = new AbortController();
  const gone = () => reads.signal.aborted;
  let done: (() => void) | undefined;
  let observer: IntersectionObserver | undefined;

  const load = async () => {
    const pdfjs = await import("pdfjs-dist/legacy/build/pdf.mjs");
    pdfjs.GlobalWorkerOptions.workerSrc = workerUrl;
    const head = await fetched(props.url, `bytes=0-${PDF_CHUNK - 1}`, reads.signal);
    const range = new pdfjs.PDFDataRangeTransport(head.size, head.bytes);
    range.requestDataRange = (begin, end) =>
      void rangeOf(props.url, begin, end, reads.signal).then(
        (bytes) => gone() || range.onDataRange(begin, bytes),
        fail,
      );
    const assets = (directory: string) => new URL(`${PDF_ASSETS}/${directory}/`, document.baseURI).href;
    const loading = pdfjs.getDocument({
      range,
      rangeChunkSize: PDF_CHUNK,
      disableAutoFetch: true,
      cMapUrl: assets("cmaps"),
      cMapPacked: true,
      standardFontDataUrl: assets("standard_fonts"),
      wasmUrl: assets("wasm"),
      iccUrl: assets("iccs"),
    });
    done = () => void loading.destroy();
    if (gone()) return done();
    const pdf = await loading.promise;
    const first = (await pdf.getPage(1)).getViewport({ scale: 1 });
    setRatio(first.height / first.width);

    const draw = async (canvas: HTMLCanvasElement) => {
      const drawn: { cancelled: boolean; cancel?: () => void } = { cancelled: false };
      drawing.set(canvas, drawn);
      const page = await pdf.getPage(Number(canvas.dataset.page));
      if (drawn.cancelled) return;
      const natural = page.getViewport({ scale: 1 });
      const viewport = page.getViewport({ scale: (canvas.clientWidth * devicePixelRatio) / natural.width });
      canvas.style.aspectRatio = `${natural.width} / ${natural.height}`;
      canvas.width = viewport.width;
      canvas.height = viewport.height;
      const task = page.render({ canvas, viewport });
      drawn.cancel = () => task.cancel();
      await task.promise.catch((error: unknown) => {
        if (!(error instanceof pdfjs.RenderingCancelledException)) throw error;
      });
    };
    const drop = (canvas: HTMLCanvasElement) => {
      const drawn = drawing.get(canvas);
      if (drawn === undefined) return;
      drawn.cancelled = true;
      drawn.cancel?.();
      drawing.delete(canvas);
      canvas.width = canvas.height = 0;
    };
    observer = new IntersectionObserver(
      (entries) => {
        for (const { target, isIntersecting } of entries) {
          const canvas = target as HTMLCanvasElement;
          if (!isIntersecting) drop(canvas);
          else if (!drawing.has(canvas)) draw(canvas).catch(fail);
        }
      },
      { root: props.scroller(), rootMargin: "100% 0px" },
    );
    setPages(Array.from({ length: pdf.numPages }, (_, at) => at + 1));
  };
  const [failed, setFailed] = createSignal<string>();
  /** The first failure shows; the teardown errors it sets off do not replace it. */
  const fail = (error: unknown) => gone() || setFailed((first) => first ?? String(error));
  onMount(() => load().catch(fail));
  onCleanup(() => {
    reads.abort();
    observer?.disconnect();
    done?.();
  });

  return (
    <Show
      when={failed()}
      fallback={
        <div class="rendered-pdf">
          <For each={pages()}>
            {(page) => (
              <canvas
                data-page={page}
                style={{ "aspect-ratio": `1 / ${ratio()}` }}
                ref={(canvas) => observer?.observe(canvas)}
              />
            )}
          </For>
        </div>
      }
    >
      {(problem) => <div class="empty">{problem()}</div>}
    </Show>
  );
}

/** A file that shows with no code: rendered from the task's directory, or as a line naming it with its size. A change to the
 * file comes as a new `view`, which mounts this anew. */
export function Media(props: { files: TaskFiles; path: string; view: MediaView }) {
  let view!: HTMLDivElement;
  const [undecodable, setUndecodable] = createSignal(false);
  const line = () => `${props.view.as === "large" ? "too large to open" : "binary file"}, ${sizeOf(props.view.size)}`;
  /** A source the webview cannot decode shows as binary; a source it cannot reach keeps its player, which says so. */
  const playable = (player: HTMLMediaElement) => {
    const code = player.error?.code;
    if (code === MediaError.MEDIA_ERR_DECODE || code === MediaError.MEDIA_ERR_SRC_NOT_SUPPORTED) setUndecodable(true);
  };
  const url = () => ("load" in props.view ? `${props.files.urlOf(props.path)}?load=${props.view.load}` : "");
  return (
    <div class="rendered over-editor" ref={view} tabIndex={-1}>
      <Switch fallback={<div class="empty">{line()}</div>}>
        <Match when={props.view.as === "image" && !undecodable()}>
          <div class="rendered-image">
            <img alt="" src={url()} onError={() => setUndecodable(true)} />
          </div>
        </Match>
        <Match when={props.view.as === "video" && !undecodable()}>
          <div class="rendered-image">
            <video controls preload="metadata" src={url()} onError={(event) => playable(event.currentTarget)} />
          </div>
        </Match>
        <Match when={props.view.as === "audio" && !undecodable()}>
          <div class="rendered-audio">
            <audio controls preload="metadata" src={url()} onError={(event) => playable(event.currentTarget)} />
          </div>
        </Match>
        <Match when={props.view.as === "pdf"}>
          <Pdf url={url()} scroller={() => view} />
        </Match>
      </Switch>
    </div>
  );
}
