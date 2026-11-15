/** From `http://` or `https://` to whitespace, quotes or angle brackets; parentheses count only in balanced pairs. */
const URL_PATTERN = /https?:\/\/(?:[^\s"'<>`()]|\([^\s"'<>`()]*\))*(?:[^\s"'<>`().,:;!?*~\]}]|\([^\s"'<>`()]*\))/g;

/** Whether a URL's host is `localhost` or a loopback address. */
export const onLoopback = (url: string) => {
  if (!URL.canParse(url)) return false;
  const host = new URL(url).hostname;
  return host === "localhost" || host === "[::1]" || /^127(\.\d{1,3}){3}$/.test(host);
};

export const urlsIn = (text: string) =>
  [...text.matchAll(URL_PATTERN)].map((match) => ({ url: match[0], at: match.index }));
