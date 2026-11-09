pub mod account;
pub mod device;
pub mod frames;
pub mod local;
pub mod ops;
pub mod pairing;
pub mod remote;
pub mod schema;
pub mod term;
pub mod types;

pub use frames::{
    Error, Event, FileSize, Op, Outcome, Push, Request, Response, ServerFrame, StreamHeader, line, parse_args,
    parse_request, request_from,
};

pub fn lock<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|poison| poison.into_inner())
}

/// Milliseconds since 1970.
pub fn now() -> i64 {
    let elapsed =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).expect("the clock reads after 1970");
    i64::try_from(elapsed.as_millis()).expect("milliseconds since 1970 fit in i64")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    schema::record!(Page { url: String, panel: String });
    schema::record!(Nested { task: types::TaskId, page: Page, tabs: Vec<Page> });

    impl Op for Nested {
        const NAME: &'static str = "test.nested";
        const DOC: &'static str = "";
        type Ok = ();
    }

    #[test]
    fn nested_args_name_every_missing_and_undeclared_field_at_every_depth() {
        let args = json!({"page": {"url": "u"}, "tabs": [{"url": "u", "panel": "p"}, {"panel": "p"}]});
        let missing = parse_args::<Nested>(&args).unwrap_err();
        assert_eq!(
            missing,
            Error::MissingField { fields: vec!["task".into(), "page.panel".into(), "tabs[1].url".into()] }
        );
        let args = json!({"task": "tsk_1", "page": {"url": "u", "panel": "p", "zoom": 2}, "tabs": []});
        assert_eq!(parse_args::<Nested>(&args).unwrap_err(), Error::UnknownField { fields: vec!["page.zoom".into()] });
        let args = json!({"workspace": "wsp_8f2", "title": "x", "parent": null, "state": "pinned", "checkout": "root", "harness": "claude-code", "session": "fresh"});
        assert_eq!(parse_args::<ops::TaskCreate>(&args).unwrap().parent, None);
    }

    #[test]
    fn pushes_errors_and_stream_headers_keep_their_wire_shape() {
        use types::{PtyId, ReadDir, Stream, TaskId};
        let push = Push { rev: 7, event: Event::HarnessUpdated { harness: "claude-code".into() } };
        assert_eq!(
            serde_json::to_value(push).unwrap(),
            json!({"rev": 7, "ev": "harness.updated", "harness": "claude-code"})
        );
        let error = Error::PtyLocked { holder: "laptop".into() };
        assert_eq!(serde_json::to_value(error).unwrap(), json!({"code": "pty_locked", "holder": "laptop"}));
        let read =
            Stream::Read { dir: ReadDir::Checkout(TaskId("tsk_1".into())), path: "a.mp4".into(), start: 2, len: 3 };
        let header = StreamHeader { open: read, client: "laptop".into() };
        let expected = json!({"read": {"dir": {"checkout": "tsk_1"}, "path": "a.mp4", "start": 2, "len": 3}});
        assert_eq!(serde_json::to_value(header).unwrap(), json!({"open": expected, "client": "laptop"}));
        assert_eq!(serde_json::to_value(Stream::Pty(PtyId("pty_2".into()))).unwrap(), json!({"pty": "pty_2"}));
    }
}
