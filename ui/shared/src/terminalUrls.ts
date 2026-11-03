/** From `http://` or `https://` to whitespace, quotes or angle brackets; parentheses count only in balanced pairs. */
const URL_PATTERN = /https?:\/\/(?:[^\s"'<>`()]|\([^\s"'<>`()]*\))*(?:[^\s"'<>`().,:;!?*~\]}]|\([^\s"'<>`()]*\))/g;

/** Every URL in a terminal's `text`, with the index it starts at. */
export const urlsIn = (text: string) =>
  [...text.matchAll(URL_PATTERN)].map((match) => ({ url: match[0], at: match.index }));
