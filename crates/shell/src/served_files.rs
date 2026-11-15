//! Every file a webview loads from a server comes from `octopus-file://<root>/<path>`. A root is a directory on one
//! server: a browser panel's page root, the directory of the file a page loads from, or a task's directory, which the
//! Files tab reads. Each root gets a random name this client keeps for as long as it runs, so no page names a root it
//! was not served from. A page root's name is its origin, so a page shares its origin with its own root's files alone.
//! A task's files go to the UI's own webviews alone, which may read them across origins. Android's webviews give a
//! custom scheme no origin, so there a page root is the `http` host `octopus-file-<root>.localhost`, a site of its own
//! that shares no cookie with another root's, and a task's root a path under `octopus-file.localhost`, which wry hands
//! this client as `octopus-file://localhost/<root>/<path>`. This client answers each request with the bytes its
//! server reads at `<path>` under the root, which the server keeps the path inside: the range a `Range` header asks
//! for, so a video plays and seeks, or else the whole file.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use octopus_client::CallError;
use octopus_proto::Error;
use octopus_proto::types::{ReadDir, Stream};
use percent_encoding::percent_decode_str;
use tauri::http::{Method, Request, Response, StatusCode, header};
use tauri::{Manager, Url, Wry};
use tokio::io::AsyncReadExt;

use crate::hub::Hub;
use crate::lock;
use crate::task_lists::TaskRef;

pub const SCHEME: &str = "octopus-file";
const ROOT_BYTES: usize = 16;

const ANDROID: bool = cfg!(target_os = "android");

/// The origin the app's own UI loads from.
const UI_ORIGIN: &str = if ANDROID { "http://tauri.localhost" } else { "tauri://localhost" };

/// The label of the window and of the UI's base webview, as `tauri.conf.json` declares them.
pub const BASE_WEBVIEW: &str = "main";
/// The label of the desktop's float layer, the UI's other webview.
pub const FLOAT_WEBVIEW: &str = "float";
const UI_WEBVIEWS: [&str; 2] = [BASE_WEBVIEW, FLOAT_WEBVIEW];

#[derive(Debug, Clone, PartialEq, Eq)]
struct Root {
    server: String,
    dir: ReadDir,
}

/// The roots this client has served files from, by name.
#[derive(Default)]
pub struct ServedFiles(Mutex<HashMap<String, Root>>);

impl ServedFiles {
    /// Where a webview loads `page` of `server`: a file page from its root's origin, any other page as it is.
    pub fn loaded_from(&self, server: &str, page: &Url) -> Result<Url, String> {
        if page.scheme() != "file" {
            return Ok(page.clone());
        }
        let file = page.to_file_path().map_err(|()| format!("{page} names no absolute path"))?;
        let dir = file.parent().ok_or_else(|| format!("{page} names no file"))?.display().to_string();
        let name = page.path_segments().and_then(Iterator::last).unwrap_or_default();
        let base = self.base(Root { server: server.to_string(), dir: ReadDir::Page(dir) });
        let mut loaded = Url::parse(&format!("{base}/{name}")).map_err(|error| error.to_string())?;
        loaded.set_query(page.query());
        loaded.set_fragment(page.fragment());
        Ok(loaded)
    }

    /// The URL a task's files load under, each at its path, percent-encoded, after a `/`.
    pub fn task(&self, at: TaskRef) -> String {
        self.base(Root { server: at.server, dir: ReadDir::Task(at.task) })
    }

    fn base(&self, root: Root) -> String {
        let page = matches!(root.dir, ReadDir::Page(_));
        let mut roots = lock(&self.0);
        let name = match roots.iter().find(|(_, known)| **known == root) {
            Some((name, _)) => name.clone(),
            None => {
                let mut bytes = [0u8; ROOT_BYTES];
                getrandom::fill(&mut bytes).expect("the OS gives random bytes");
                let name: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
                roots.insert(name.clone(), root);
                name
            }
        };
        match (ANDROID, page) {
            (false, _) => format!("{SCHEME}://{name}"),
            (true, true) => format!("http://{SCHEME}-{name}.localhost"),
            (true, false) => format!("http://{SCHEME}.localhost/{name}"),
        }
    }

    /// The page a webview shows at `loaded`, a file page as its `file://` URL.
    pub fn page_of(&self, loaded: &Url) -> Url {
        match self.served(loaded) {
            Some((Root { dir: ReadDir::Page(dir), .. }, path)) => {
                let mut page = Url::from_file_path(PathBuf::from(dir).join(path)).expect("a root is an absolute path");
                page.set_query(loaded.query());
                page.set_fragment(loaded.fragment());
                page
            }
            _ => loaded.clone(),
        }
    }

    /// The root `loaded` is a file of, and its path under the root.
    fn served(&self, loaded: &Url) -> Option<(Root, String)> {
        let (name, path) = named(loaded, ANDROID)?;
        let root = lock(&self.0).get(name)?.clone();
        Some((root, percent_decode_str(path).decode_utf8().ok()?.into_owned()))
    }

    pub async fn answer(
        &self,
        hub: &Hub,
        asker: &str,
        loaded: &Url,
        method: &Method,
        range: Option<&str>,
    ) -> Response<Vec<u8>> {
        let Some((root, path)) = self.served(loaded).filter(|(root, _)| serves(&root.dir, asker)) else {
            return respond(StatusCode::NOT_FOUND, format!("no root at {loaded}"));
        };
        let reply = if method == Method::OPTIONS {
            Response::builder().status(StatusCode::NO_CONTENT).body(Vec::new()).expect("a status makes a response")
        } else {
            read(hub, &root, path, range).await
        };
        readable_by(&root.dir, reply)
    }
}

/// The name of the root `loaded` is a file of, and its path under the root.
fn named(loaded: &Url, android: bool) -> Option<(&str, &str)> {
    let path = loaded.path().strip_prefix('/')?;
    match (android, loaded.scheme(), loaded.host_str()?) {
        (false, SCHEME, root) => Some((root, path)),
        (true, SCHEME, "localhost") => path.split_once('/'),
        (true, "http", host) => Some((host.strip_prefix(SCHEME)?.strip_prefix('-')?.strip_suffix(".localhost")?, path)),
        _ => None,
    }
}

/// Whether the webview labelled `asker` may load files of `dir`: a task's are the UI's alone.
fn serves(dir: &ReadDir, asker: &str) -> bool {
    matches!(dir, ReadDir::Page(_)) || UI_WEBVIEWS.contains(&asker)
}

/// Lets the app's own UI read a task's files across origins, and nothing else read any root's.
fn readable_by(dir: &ReadDir, mut reply: Response<Vec<u8>>) -> Response<Vec<u8>> {
    if let ReadDir::Task(_) = dir {
        let headers = reply.headers_mut();
        headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, header::HeaderValue::from_static(UI_ORIGIN));
        headers.insert(header::ACCESS_CONTROL_ALLOW_HEADERS, header::HeaderValue::from_static("range"));
        headers.insert(
            header::ACCESS_CONTROL_EXPOSE_HEADERS,
            header::HeaderValue::from_static("content-range, accept-ranges"),
        );
    }
    reply
}

/// The bytes of `path` under `root` that a request's `Range` header asks for, the whole file without one.
async fn read(hub: &Hub, root: &Root, path: String, range: Option<&str>) -> Response<Vec<u8>> {
    let wanted = wanted(range);
    let (start, len) = wanted.unwrap_or((0, Stream::TO_END));
    let read = match hub.connected(&root.server).await {
        Ok(connection) => connection.read(root.dir.clone(), path.clone(), start, len).await.map_err(|error| {
            let status = match &error {
                CallError::Server(Error::NotFound { .. }) => StatusCode::NOT_FOUND,
                CallError::Server(Error::InvalidArgs { .. }) => StatusCode::FORBIDDEN,
                CallError::Transport(_) | CallError::Server(_) => StatusCode::BAD_GATEWAY,
            };
            (status, hub.failure(&root.server, error))
        }),
        Err(unreachable) => Err((StatusCode::BAD_GATEWAY, unreachable)),
    };
    let (size, mut stream) = match read {
        Ok(read) => read,
        Err((status, problem)) => return respond(status, problem),
    };
    let mut bytes = Vec::new();
    if let Err(error) = stream.read_to_end(&mut bytes).await {
        return respond(StatusCode::BAD_GATEWAY, format!("read {path}: {error}"));
    }
    let (status, range) = match (wanted, bytes.len() as u64) {
        (None, _) => (StatusCode::OK, None),
        (Some(_), 0) => (StatusCode::RANGE_NOT_SATISFIABLE, Some(format!("bytes */{size}"))),
        (Some(_), sent) => (StatusCode::PARTIAL_CONTENT, Some(format!("bytes {start}-{}/{size}", start + sent - 1))),
    };
    let mut reply = response(status, content_type(&path)).header(header::ACCEPT_RANGES, "bytes");
    if let Some(range) = range {
        reply = reply.header(header::CONTENT_RANGE, range);
    }
    reply.body(bytes).expect("a status, its headers and a body make a response")
}

/// The most bytes one ranged reply carries; the webview asks again for the rest.
const RANGE_BYTES: u64 = 2 << 20;

/// The first byte and the byte count a `Range` header asks for, at most `RANGE_BYTES`. A suffix, several ranges, or a
/// header that parses as none of them reads as no header, which asks for the whole file.
fn wanted(range: Option<&str>) -> Option<(u64, u64)> {
    let (first, last) = range?.strip_prefix("bytes=")?.split_once('-')?;
    let start: u64 = first.trim().parse().ok()?;
    let len = match last.trim() {
        "" => RANGE_BYTES,
        last => last.parse::<u64>().ok()?.checked_sub(start)?.saturating_add(1).min(RANGE_BYTES),
    };
    Some((start, len))
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
        Some("bmp") => "image/bmp",
        Some("tif" | "tiff") => "image/tiff",
        Some("heic") => "image/heic",
        Some("wasm") => "application/wasm",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        Some("ttf") => "font/ttf",
        Some("otf") => "font/otf",
        Some("pdf") => "application/pdf",
        Some("xml") => "application/xml",
        Some("mp4") => "video/mp4",
        Some("m4v") => "video/x-m4v",
        Some("mov") => "video/quicktime",
        Some("webm") => "video/webm",
        Some("mp3") => "audio/mpeg",
        Some("m4a") => "audio/mp4",
        Some("aac") => "audio/aac",
        Some("flac") => "audio/flac",
        Some("wav") => "audio/wav",
        Some("ogg" | "oga") => "audio/ogg",
        Some("opus") => "audio/opus",
        _ => "text/plain; charset=utf-8",
    }
}

fn response(status: StatusCode, kind: &str) -> tauri::http::response::Builder {
    Response::builder().status(status).header(header::CONTENT_TYPE, kind).header(header::CACHE_CONTROL, "no-store")
}

fn respond(status: StatusCode, problem: String) -> Response<Vec<u8>> {
    response(status, "text/plain").body(problem.into_bytes()).expect("a status, two headers and a body make a response")
}

/// Answers the webviews' `octopus-file` requests.
pub fn protocol(builder: tauri::plugin::Builder<Wry>) -> tauri::plugin::Builder<Wry> {
    #[cfg(target_os = "linux")]
    let builder = builder.on_webview_ready(cross_origin);
    builder.register_asynchronous_uri_scheme_protocol(SCHEME, |context, request: Request<Vec<u8>>, responder| {
        let app = context.app_handle();
        let asker = context.webview_label().to_owned();
        let hub = Arc::clone(app.state::<Arc<Hub>>().inner());
        let served = Arc::clone(app.state::<Arc<ServedFiles>>().inner());
        let loaded = Url::parse(&request.uri().to_string());
        let method = request.method().clone();
        let range = request.headers().get(header::RANGE).and_then(|range| range.to_str().ok()).map(str::to_owned);
        tauri::async_runtime::spawn(async move {
            responder.respond(match loaded {
                Ok(loaded) => served.answer(&hub, &asker, &loaded, &method, range.as_deref()).await,
                Err(error) => respond(StatusCode::BAD_REQUEST, error.to_string()),
            });
        });
    })
}

/// WebKitGTK lets a page read a custom scheme's replies across origins only once the scheme is marked for it.
#[cfg(target_os = "linux")]
fn cross_origin(webview: tauri::Webview<Wry>) {
    if let Err(error) = webview.with_webview(|shown| {
        use webkit2gtk::{SecurityManagerExt, WebContextExt, WebViewExt};
        if let Some(security) = shown.inner().context().and_then(|context| context.security_manager()) {
            security.register_uri_scheme_as_cors_enabled(SCHEME);
        }
    }) {
        eprintln!("octopus app: {}: {error}", webview.label());
    }
}

#[cfg(test)]
mod tests {
    use octopus_proto::types::TaskId;

    use super::*;

    #[test]
    fn a_range_header_asks_for_a_bounded_run_of_bytes() {
        assert_eq!(wanted(Some("bytes=0-1")), Some((0, 2)));
        assert_eq!(wanted(Some("bytes=100-")), Some((100, RANGE_BYTES)));
        assert_eq!(wanted(Some("bytes=0-99999999")), Some((0, RANGE_BYTES)));
        assert_eq!(wanted(Some("bytes=0-18446744073709551615")), Some((0, RANGE_BYTES)));
        for whole in [None, Some("bytes=-500"), Some("bytes=0-1,4-5"), Some("bytes=9-3"), Some("items=0-1")] {
            assert_eq!(wanted(whole), None, "{whole:?}");
        }
    }

    fn url(text: &str) -> Url {
        Url::parse(text).expect("a URL")
    }

    #[test]
    fn a_page_shares_its_origin_with_its_own_root_alone_and_reads_back_as_its_file_url() {
        let served = ServedFiles::default();
        let page = url("file:///tmp/my%20site/index.html?tab=2#top");
        let loaded = served.loaded_from("mac mini", &page).expect("a file page");
        assert_eq!(served.page_of(&loaded), page);
        let style = loaded.join("sub%20dir/../style.css").expect("a URL");
        assert_eq!(served.page_of(&style).as_str(), "file:///tmp/my%20site/style.css");
        let origin = |server: &str, page: &str| {
            served.loaded_from(server, &url(page)).expect("a file page").host_str().expect("an origin").to_string()
        };
        let site = origin("mac mini", "file:///tmp/site/index.html");
        assert_eq!(origin("mac mini", "file:///tmp/site/other.html"), site, "one root, one origin");
        assert_ne!(origin("mac mini", "file:///tmp/site/sub/page.html"), site, "a deeper root is a root of its own");
        assert_ne!(origin("mac mini", "file:///Users/me/.ssh/key.html"), site);
        assert_ne!(origin("laptop", "file:///tmp/site/index.html"), site, "another server's same directory");
        let climbed = url(&format!("{SCHEME}://{site}/../../Users/me/.ssh/id_ed25519"));
        let (root, path) = served.served(&climbed).expect("the root's own origin");
        assert_eq!((root.dir, path.as_str()), (ReadDir::Page("/tmp/site".into()), "Users/me/.ssh/id_ed25519"));
        let guessed = url(&format!("{SCHEME}://{}/id_ed25519", "0".repeat(2 * ROOT_BYTES)));
        assert_eq!(served.served(&guessed), None, "an origin this client never gave names nothing");
    }

    #[test]
    fn a_tasks_directory_is_a_root_of_its_own_that_the_ui_alone_reads_across_origins() {
        let served = ServedFiles::default();
        let at = |server: &str| TaskRef { server: server.to_string(), task: TaskId("tsk_1".into()) };
        let directory = served.task(at("mac mini"));
        assert_eq!(served.task(at("mac mini")), directory, "one task, one root");
        assert_ne!(served.task(at("laptop")), directory, "another server's task of the same id");
        let page = served.loaded_from("mac mini", &url("file:///Users/me/repo/index.html")).expect("a file page");
        assert_ne!(page.host_str(), url(&directory).host_str(), "a page is never a task's origin");
        let (root, path) =
            served.served(&url(&format!("{directory}/docs/my%20plan.pdf"))).expect("the task's directory");
        assert_eq!((root.dir, path.as_str()), (ReadDir::Task(TaskId("tsk_1".into())), "docs/my plan.pdf"));

        let allowed = |dir: &ReadDir| {
            let reply = readable_by(dir, respond(StatusCode::OK, String::new()));
            reply
                .headers()
                .get(header::ACCESS_CONTROL_ALLOW_ORIGIN)
                .map(|origin| origin.to_str().expect("ASCII").to_owned())
        };
        assert_eq!(allowed(&ReadDir::Task(TaskId("tsk_1".into()))).as_deref(), Some(UI_ORIGIN));
        assert_eq!(allowed(&ReadDir::Page("/tmp/site".into())), None, "a page root is its own origin's alone");
        assert!(serves(&ReadDir::Task(TaskId("tsk_1".into())), BASE_WEBVIEW));
        assert!(!serves(&ReadDir::Task(TaskId("tsk_1".into())), "browser-0"), "a browser panel's webview");
        assert!(serves(&ReadDir::Page("/tmp/site".into()), "browser-0"));
    }

    #[test]
    fn android_names_a_page_root_by_its_host_and_a_tasks_root_by_the_path_wry_hands_over() {
        let on_android = |text: &str| named(&url(text), true).map(|(root, path)| format!("{root} {path}"));
        assert_eq!(
            on_android("http://octopus-file-ab12.localhost/sub/page.html").as_deref(),
            Some("ab12 sub/page.html")
        );
        assert_eq!(on_android("octopus-file://localhost/ab12/docs/plan.md").as_deref(), Some("ab12 docs/plan.md"));
        assert_eq!(on_android("http://example.com/ab12/plan.md"), None);
    }
}
