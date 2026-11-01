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
    Error, Event, Op, Outcome, Push, Request, Response, ServerFrame, StreamHeader, line, parse_args, parse_request,
    request_from,
};

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_request_missing_fields_names_every_absent_field() {
        let error = parse_args::<ops::TaskCreate>(&json!({"workspace": "wsp_8f2", "title": "x"})).unwrap_err();
        let missing = ["parent", "state", "checkout", "harness", "session"].map(String::from).to_vec();
        assert_eq!(error, Error::MissingField { fields: missing });
    }

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
        let args = json!({"workspace": "wsp_8f2", "title": "x", "parent": null, "state": "pinned", "checkout": "root", "harness": "claude-code", "session": null});
        assert_eq!(parse_args::<ops::TaskCreate>(&args).unwrap().parent, None);
    }

    #[test]
    fn an_undeclared_field_is_rejected() {
        let error = parse_args::<ops::TaskFocus>(&json!({"task": "tsk_1", "args": [], "force": true})).unwrap_err();
        assert_eq!(error, Error::UnknownField { fields: vec!["force".into()] });
    }

    #[test]
    fn a_push_carries_its_event_name_beside_the_rev() {
        let push = Push { rev: 7, event: Event::HarnessUpdated { harness: "claude-code".into() } };
        let wire = serde_json::to_value(&push).unwrap();
        assert_eq!(wire, json!({"rev": 7, "ev": "harness.updated", "harness": "claude-code"}));
        assert_eq!(serde_json::from_value::<Push>(wire).unwrap(), push);
    }

    #[test]
    fn a_stream_name_round_trips() {
        use types::{PtyId, Stream, TaskId};
        let file = Stream::File { task: TaskId("tsk_1".into()), path: "src/main.rs".into() };
        assert_eq!(file.name(), "file:tsk_1/src/main.rs");
        assert_eq!(Stream::parse(&file.name()), Some(file));
        let upload = Stream::Upload { task: TaskId("tsk_1".into()), bytes: 4, path: "logo.png".into() };
        assert_eq!(Stream::parse(&upload.name()), Some(upload));
        assert_eq!(Stream::parse("pty:pty_2"), Some(Stream::Pty(PtyId("pty_2".into()))));
        assert_eq!(Stream::parse(&Stream::Tunnel(5173).name()), Some(Stream::Tunnel(5173)));
        assert_eq!(Stream::parse("tunnel:http"), None);
    }

    #[test]
    fn errors_serialize_with_their_code() {
        let wire = serde_json::to_value(Error::PtyLocked { holder: "laptop".into() }).unwrap();
        assert_eq!(wire, json!({"code": "pty_locked", "holder": "laptop"}));
    }
}
