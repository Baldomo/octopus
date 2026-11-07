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
    }

    #[test]
    fn a_null_parent_is_present_not_missing() {
        let args = json!({"workspace": "wsp_8f2", "title": "x", "parent": null, "state": "pinned", "checkout": "root", "harness": "claude-code", "session": "fresh"});
        assert_eq!(parse_args::<ops::TaskCreate>(&args).unwrap().parent, None);
    }

    #[test]
    fn a_push_carries_its_event_name_beside_the_rev_and_an_error_its_code() {
        let push = Push { rev: 7, event: Event::HarnessUpdated { harness: "claude-code".into() } };
        let wire = serde_json::to_value(&push).unwrap();
        assert_eq!(wire, json!({"rev": 7, "ev": "harness.updated", "harness": "claude-code"}));
        assert_eq!(serde_json::from_value::<Push>(wire).unwrap(), push);
        let wire = serde_json::to_value(Error::PtyLocked { holder: "laptop".into() }).unwrap();
        assert_eq!(wire, json!({"code": "pty_locked", "holder": "laptop"}));
    }

    #[test]
    fn a_stream_header_names_its_stream_as_json() {
        use types::{PtyId, ReadDir, Stream, TaskId};
        let read =
            Stream::Read { dir: ReadDir::Checkout(TaskId("tsk_1".into())), path: "a.mp4".into(), start: 2, len: 3 };
        let header = StreamHeader { open: read, client: "laptop".into() };
        let wire = serde_json::to_value(&header).unwrap();
        let expected = json!({"read": {"dir": {"checkout": "tsk_1"}, "path": "a.mp4", "start": 2, "len": 3}});
        assert_eq!(wire, json!({"open": expected, "client": "laptop"}));
        assert_eq!(serde_json::from_value::<StreamHeader>(wire).unwrap(), header);
        assert_eq!(serde_json::to_value(Stream::Pty(PtyId("pty_2".into()))).unwrap(), json!({"pty": "pty_2"}));
        let stray = json!({"open": {"tunnel": 5173}, "client": "laptop", "zoom": 1});
        assert!(serde_json::from_value::<StreamHeader>(stray).is_err());
    }
}
