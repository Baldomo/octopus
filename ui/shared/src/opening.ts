export const extensionOf = (path: string) => {
  const name = path.slice(path.lastIndexOf("/") + 1);
  return name.includes(".") ? name.slice(name.lastIndexOf(".") + 1).toLowerCase() : "";
};

/** Text with a rendered view beside its code. */
const RENDERINGS = new Map<string, "markdown" | "page" | "image">([
  ["md", "markdown"],
  ["markdown", "markdown"],
  ["html", "page"],
  ["htm", "page"],
  ["svg", "image"],
]);
export const renderingOf = (path: string) => RENDERINGS.get(extensionOf(path));

export type MediaKind = "image" | "pdf" | "video" | "audio";
const MEDIA = new Map<string, MediaKind>([
  ...["png", "jpg", "jpeg", "gif", "webp", "avif", "bmp", "ico", "tif", "tiff", "heic"].map(
    (extension) => [extension, "image"] as const,
  ),
  ["pdf", "pdf"],
  ...["mp4", "m4v", "mov", "webm"].map((extension) => [extension, "video"] as const),
  ...["mp3", "m4a", "aac", "wav", "flac", "ogg", "oga", "opus"].map((extension) => [extension, "audio"] as const),
]);

/** The bytes that tell text from binary, as git reads them. */
export const SNIFF_BYTES = 8 << 10;
/** The editor holds a whole text file as a string, twice its bytes, so a bigger one stays unopened. */
export const TEXT_BYTES = 16 << 20;
/** The webview reads a whole image before it decodes it, so a bigger one stays unshown. */
export const IMAGE_BYTES = 64 << 20;
/** The PDF reader keeps a buffer of the whole file's size, though it reads only the ranges it draws. */
export const PDF_BYTES = 1 << 30;
/** The bytes the PDF reader asks for at a time. */
export const PDF_CHUNK = 1 << 20;
/** Where the build puts the files the PDF reader loads as it needs them: CMaps, standard fonts and decoders. */
export const PDF_ASSETS = "pdfjs";

/** How a file opens: as code, as media its view loads by itself, or as a line naming what it is and its size. */
export type Opening = "text" | MediaKind | "binary" | "large";

/** The file's text, when `bytes` are UTF-8 with no NUL; `whole` says whether they end where the file does. */
export function textOf(bytes: Uint8Array, whole = true) {
  if (bytes.includes(0)) return undefined;
  try {
    return new TextDecoder("utf-8", { fatal: true }).decode(bytes, { stream: !whole });
  } catch {
    return undefined;
  }
}

/** Decides from the file's name, its size and its first bytes, `head`. */
export function openingOf(path: string, size: number, head: Uint8Array): Opening {
  const media = MEDIA.get(extensionOf(path));
  if ((media === "image" && size > IMAGE_BYTES) || (media === "pdf" && size > PDF_BYTES)) return "large";
  if (media !== undefined) return media;
  if (textOf(head, head.length >= size) === undefined) return "binary";
  return size > TEXT_BYTES ? "large" : "text";
}

const UNITS = ["bytes", "KB", "MB", "GB", "TB"];

export function sizeOf(bytes: number) {
  const power = Math.min(Math.floor(Math.log(Math.max(bytes, 1)) / Math.log(1024)), UNITS.length - 1);
  const value = bytes / 1024 ** power;
  return power === 0 ? `${bytes} bytes` : `${value < 10 ? value.toFixed(1) : Math.round(value)} ${UNITS[power]}`;
}

/** The file's bytes that `range` asks for, or all of them, and the file's size; a failure rejects with its reason. */
export async function fetched(url: string, range?: string, signal?: AbortSignal) {
  const reply = await fetch(url, { headers: range === undefined ? {} : { Range: range }, signal });
  if (!reply.ok && reply.status !== 416) throw await reply.text();
  const bytes = new Uint8Array(await reply.arrayBuffer());
  const total = reply.headers.get("Content-Range")?.split("/")[1];
  return { size: total === undefined ? bytes.length : Number(total), bytes };
}

/** The file's bytes from `begin` up to `end`, asked for again past each reply, which the server may cut short. */
export async function rangeOf(url: string, begin: number, end: number, signal: AbortSignal) {
  const bytes = new Uint8Array(end - begin);
  for (let at = begin; at < end;) {
    const { bytes: part } = await fetched(url, `bytes=${at}-${end - 1}`, signal);
    if (part.length === 0) throw `${url} ends before byte ${end}`;
    bytes.set(part, at - begin);
    at += part.length;
  }
  return bytes;
}
