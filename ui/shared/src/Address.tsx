/** A page's address in three runs: a file's directories, then its name or a web page's host, then what follows. */
function addressOf(url: string) {
  try {
    const parsed = new URL(url);
    if (parsed.protocol === "file:") {
      const path = decodeURIComponent(parsed.pathname);
      const cut = path.lastIndexOf("/") + 1;
      return { lead: path.slice(0, cut), main: path.slice(cut), rest: `${parsed.search}${parsed.hash}` };
    }
    const rest = `${parsed.pathname === "/" ? "" : parsed.pathname}${parsed.search}${parsed.hash}`;
    return { lead: "", main: parsed.host, rest };
  } catch {
    return { lead: "", main: url, rest: "" };
  }
}

export function Address(props: { url: string }) {
  const parts = () => addressOf(props.url);
  return (
    <span class="name address">
      <span class="lead">{parts().lead}</span>
      <b>{parts().main}</b>
      <span class="rest">{parts().rest}</span>
    </span>
  );
}
