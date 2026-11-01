//! A browser panel's file page loads in a webview from `octopus-page://<origin>/<path>`. Each page root, the
//! directory of the file a page loads from on its server, gets an `<origin>` of its own: a random name this client
//! keeps for as long as it runs. A page is then the same origin as its root's other files alone, and no page names
//! another root's or another server's files, since it cannot guess their origin. This client answers each request
//! with the bytes its server reads at `<path>` under the root, which the server keeps the path inside.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use octopus_proto::types::Stream;
use percent_encoding::percent_decode_str;
use tauri::http::{Request, Response, StatusCode, header};
use tauri::{Manager, Runtime, Url};
use tokio::io::AsyncReadExt;

use crate::hub::Hub;
use crate::lock;

const SCHEME: &str = "octopus-page";
const ORIGIN_BYTES: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq)]
struct Root {
    server: String,
    dir: PathBuf,
}

/// The page roots this client has loaded file pages from, by origin.
#[derive(Default)]
pub struct Pages(Mutex<HashMap<String, Root>>);

impl Pages {
    /// Where a webview loads `page` of `server`: a file page from its root's origin, any other page as it is.
    pub fn loaded_from(&self, server: &str, page: &Url) -> Result<Url, String> {
        if page.scheme() != "file" {
            return Ok(page.clone());
        }
        if cfg!(target_os = "android") {
            return Err(format!(
                "{page}: Android's webview serves no page an origin of its own, so it shows no file page"
            ));
        }
        let file = page.to_file_path().map_err(|()| format!("{page} names no absolute path"))?;
        let dir = file.parent().ok_or_else(|| format!("{page} names no file"))?.to_path_buf();
        let name = page.path_segments().and_then(Iterator::last).unwrap_or_default();
        let origin = self.origin(Root { server: server.to_string(), dir });
        let mut loaded = Url::parse(&format!("{SCHEME}://{origin}/{name}")).map_err(|error| error.to_string())?;
        loaded.set_query(page.query());
        loaded.set_fragment(page.fragment());
        Ok(loaded)
    }

    fn origin(&self, root: Root) -> String {
        let mut roots = lock(&self.0);
        if let Some((origin, _)) = roots.iter().find(|(_, known)| **known == root) {
            return origin.clone();
        }
        let mut name = [0u8; ORIGIN_BYTES];
        getrandom::fill(&mut name).expect("the OS gives random bytes");
        let origin: String = name.iter().map(|byte| format!("{byte:02x}")).collect();
        roots.insert(origin.clone(), root);
        origin
    }

    /// The page a webview shows at `loaded`, a file page as its `file://` URL.
    pub fn page_of(&self, loaded: &Url) -> Url {
        match self.served(loaded) {
            Some((root, path)) => {
                let mut page = Url::from_file_path(root.dir.join(path)).expect("a root is an absolute path");
                page.set_query(loaded.query());
                page.set_fragment(loaded.fragment());
                page
            }
            None => loaded.clone(),
        }
    }

    /// The root `loaded` is a file of, and its path under the root.
    fn served(&self, loaded: &Url) -> Option<(Root, String)> {
        if loaded.scheme() != SCHEME {
            return None;
        }
        let root = lock(&self.0).get(loaded.host_str()?)?.clone();
        let path = percent_decode_str(loaded.path().strip_prefix('/')?).decode_utf8().ok()?.into_owned();
        Some((root, path))
    }

    async fn read(&self, hub: &Hub, loaded: &Url) -> Response<Vec<u8>> {
        let Some((root, path)) = self.served(loaded) else {
            return respond(StatusCode::NOT_FOUND, "text/plain", format!("no page root at {loaded}").into_bytes());
        };
        let stream = Stream::Page { root: root.dir.display().to_string(), path: path.clone() };
        let opened = match hub.connection(&root.server) {
            Ok(connection) => connection.open_data_stream(&stream).await.map_err(|error| {
                let status = match &error {
                    octopus_client::CallError::Server(octopus_proto::Error::NotFound { .. }) => StatusCode::NOT_FOUND,
                    octopus_client::CallError::Server(octopus_proto::Error::InvalidArgs { .. }) => {
                        StatusCode::FORBIDDEN
                    }
                    octopus_client::CallError::Transport(_) | octopus_client::CallError::Server(_) => {
                        StatusCode::BAD_GATEWAY
                    }
                };
                (status, hub.failure(&root.server, error))
            }),
            Err(unreachable) => Err((StatusCode::BAD_GATEWAY, unreachable)),
        };
        let mut bytes = Vec::new();
        match opened {
            Ok(mut stream) => match stream.read_to_end(&mut bytes).await {
                Ok(_) => respond(StatusCode::OK, content_type(&path), bytes),
                Err(error) => {
                    respond(StatusCode::BAD_GATEWAY, "text/plain", format!("read {path}: {error}").into_bytes())
                }
            },
            Err((status, problem)) => respond(status, "text/plain", problem.into_bytes()),
        }
    }
}

fn content_type(path: &str) -> &'static str {
    let extension = path.rsplit_once('.').map(|(_, extension)| extension.to_ascii_lowercase());
    match extension.as_deref() {
        Some("html" | "htm") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js" | "mjs" | "cjs") => "text/javascript; charset=utf-8",
        Some("json" | "map") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("avif") => "image/avif",
        Some("ico") => "image/x-icon",
        Some("wasm") => "application/wasm",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        Some("ttf") => "font/ttf",
        Some("otf") => "font/otf",
        Some("pdf") => "application/pdf",
        Some("xml") => "application/xml",
        Some("mp4") => "video/mp4",
        Some("webm") => "video/webm",
        Some("mp3") => "audio/mpeg",
        Some("wav") => "audio/wav",
        _ => "text/plain; charset=utf-8",
    }
}

fn respond(status: StatusCode, kind: &str, body: Vec<u8>) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, kind)
        .header(header::CACHE_CONTROL, "no-store")
        .body(body)
        .expect("a status, two headers and a body make a response")
}

/// Answers the webviews' `octopus-page` requests.
pub fn protocol<R: Runtime>(builder: tauri::plugin::Builder<R>) -> tauri::plugin::Builder<R> {
    builder.register_asynchronous_uri_scheme_protocol(SCHEME, |context, request: Request<Vec<u8>>, responder| {
        let app = context.app_handle();
        let (hub, pages) = (Arc::clone(app.state::<Arc<Hub>>().inner()), Arc::clone(app.state::<Arc<Pages>>().inner()));
        let loaded = Url::parse(&request.uri().to_string());
        tauri::async_runtime::spawn(async move {
            responder.respond(match loaded {
                Ok(loaded) => pages.read(&hub, &loaded).await,
                Err(error) => respond(StatusCode::BAD_REQUEST, "text/plain", error.to_string().into_bytes()),
            });
        });
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(text: &str) -> Url {
        Url::parse(text).expect("a URL")
    }

    #[test]
    fn a_file_page_loads_from_its_roots_origin_and_reads_back_as_the_same_file_url() {
        let pages = Pages::default();
        let page = url("file:///tmp/my%20site/index.html?tab=2#top");
        let loaded = pages.loaded_from("mac mini", &page).expect("a file page");
        let origin = loaded.host_str().expect("an origin").to_string();
        assert_eq!((origin.len(), loaded.path()), (2 * ORIGIN_BYTES, "/index.html"));
        assert_eq!(pages.page_of(&loaded), page);
        let style = loaded.join("sub%20dir/../style.css").expect("a URL");
        assert_eq!(pages.page_of(&style).as_str(), "file:///tmp/my%20site/style.css");
        let web = url("http://localhost:5173/");
        assert_eq!((pages.loaded_from("mac mini", &web), pages.page_of(&web)), (Ok(web.clone()), web));
    }

    #[test]
    fn a_page_shares_its_origin_with_its_own_root_alone() {
        let pages = Pages::default();
        let origin = |server: &str, page: &str| {
            pages.loaded_from(server, &url(page)).expect("a file page").host_str().expect("an origin").to_string()
        };
        let site = origin("mac mini", "file:///tmp/site/index.html");
        assert_eq!(origin("mac mini", "file:///tmp/site/other.html"), site, "one root, one origin");
        assert_ne!(origin("mac mini", "file:///tmp/site/sub/page.html"), site, "a deeper root is a root of its own");
        assert_ne!(origin("mac mini", "file:///Users/me/.ssh/key.html"), site);
        assert_ne!(origin("laptop", "file:///tmp/site/index.html"), site, "another server's same directory");
        let climbed = url(&format!("{SCHEME}://{site}/../../Users/me/.ssh/id_ed25519"));
        let (root, path) = pages.served(&climbed).expect("the root's own origin");
        assert_eq!((root.dir, path.as_str()), (PathBuf::from("/tmp/site"), "Users/me/.ssh/id_ed25519"));
        let guessed = url(&format!("{SCHEME}://{}/id_ed25519", "0".repeat(2 * ORIGIN_BYTES)));
        assert_eq!(pages.served(&guessed), None, "an origin this client never gave names nothing");
    }
}
