//! The page tidewatch serves on the demo computer's port 4100, answered over the tunnel stream with no socket behind it.

use tokio::io::{AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};

pub(super) const PORT: u16 = 4100;
/// A request's head past this is read no further.
const HEAD_BYTES: usize = 16 * 1024;

pub(super) fn page(path: &str) -> String {
    format!("http://localhost:{PORT}{path}")
}

const STYLE: &str = "<style>
body { font: 16px/1.5 -apple-system, system-ui, sans-serif; margin: 0; color: #12303a; background: #f4f8f9; }
header { padding: 20px 20px 4px; } h1 { margin: 0; font-size: 26px; } .sub { color: #5d7c85; margin: 0; }
main { padding: 8px 20px 28px; } table { border-collapse: collapse; width: 100%; margin: 12px 0 20px; }
td { padding: 8px 4px; border-bottom: 1px solid #dbe6e9; } .high { color: #1f6f8b; font-weight: 600; } .low { color: #7f9ba3; }
h2 { font-size: 15px; text-transform: uppercase; letter-spacing: .06em; color: #5d7c85; margin: 18px 0 6px; }
svg { width: 100%; height: auto; background: #fff; border-radius: 12px; }
polyline { fill: none; stroke: #1f6f8b; stroke-width: 2.5; } circle.high { fill: #1f6f8b; } circle.low { fill: #9fb7be; }
.pass { color: #2e7d4f; } li { padding: 6px 0; border-bottom: 1px solid #dbe6e9; list-style: none; } ul { padding: 0; }
</style>";

/// Heights of the week's tides, two highs and two lows a day.
const WEEK: [f32; 28] = [
    4.1, 0.9, 4.3, 0.7, 4.0, 1.0, 4.2, 0.8, 3.8, 1.2, 3.9, 1.1, 3.6, 1.4, 3.7, 1.3, 3.5, 1.5, 3.6, 1.4, 3.6, 1.3, 3.8,
    1.2, 3.9, 1.1, 4.1, 0.9,
];

fn chart() -> String {
    let (width, height) = (640.0, 180.0);
    let step = width / (WEEK.len() - 1) as f32;
    let y = |metres: f32| height - 10.0 - metres / 4.5 * (height - 20.0);
    let points: Vec<String> =
        WEEK.iter().enumerate().map(|(at, m)| format!("{:.1},{:.1}", at as f32 * step, y(*m))).collect();
    let marks: String = WEEK
        .iter()
        .enumerate()
        .map(|(at, m)| {
            let kind = if at % 2 == 0 { "high" } else { "low" };
            format!("<circle class=\"{kind}\" cx=\"{:.1}\" cy=\"{:.1}\" r=\"4\"/>", at as f32 * step, y(*m))
        })
        .collect();
    format!("<svg viewBox=\"-8 0 656 {height}\"><polyline points=\"{}\"/>{marks}</svg>", points.join(" "))
}

fn tides() -> String {
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, \
         initial-scale=1\"><title>tidewatch · Saltmarsh Cove</title>{STYLE}</head><body><header><h1>Saltmarsh \
         Cove</h1><p class=\"sub\">Tides for today, Monday</p></header><main><table>\
         <tr class=\"high\"><td>04:12</td><td>high</td><td>4.1 m</td></tr>\
         <tr class=\"low\"><td>10:31</td><td>low</td><td>0.9 m</td></tr>\
         <tr class=\"high\"><td>16:40</td><td>high</td><td>4.3 m</td></tr>\
         <tr class=\"low\"><td>22:58</td><td>low</td><td>0.7 m</td></tr></table>\
         <h2>This week</h2>{}</main></body></html>",
        chart()
    )
}

fn tests() -> String {
    let passed = ["groups tides by day", "keeps each tide's offset", "reads a bare time as +00:00", "skips blank rows"];
    let items: String = passed.iter().map(|name| format!("<li><span class=\"pass\">✓</span> {name}</li>")).collect();
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, \
         initial-scale=1\"><title>tidewatch · tests</title>{STYLE}</head><body><header><h1>4 passed</h1><p \
         class=\"sub\">tests/forecast.test.js</p></header><main><ul>{items}</ul></main></body></html>"
    )
}

/// Answers one HTTP request read from `stream`, then closes it.
pub(super) async fn serve<S: tokio::io::AsyncRead + AsyncWrite + Unpin>(stream: S) {
    let mut stream = BufReader::new(stream);
    let mut head = String::new();
    while !head.ends_with("\r\n\r\n") && head.len() < HEAD_BYTES {
        match stream.read_line(&mut head).await {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
    }
    let path = head.split_whitespace().nth(1).and_then(|target| target.split(['?', '#']).next());
    let (status, body) = match path {
        Some("/" | "/index.html") => ("200 OK", tides()),
        Some("/tests") => ("200 OK", tests()),
        Some(_) => ("404 Not Found", "not found".to_string()),
        None => ("400 Bad Request", "bad request".to_string()),
    };
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: \
         no-store\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let stream = stream.get_mut();
    if stream.write_all(response.as_bytes()).await.is_ok() {
        let _browser_gone = stream.shutdown().await;
    }
}
