pub mod frames;
pub mod local;
pub mod ops;
pub mod types;

pub use frames::{
    Error, Event, Op, Outcome, Push, Request, Response, ServerFrame, StreamHeader, line, parse_args, parse_request,
};

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_request_missing_fields_names_every_absent_field() {
        let error = parse_args::<ops::TaskCreate>(&json!({"workspace": "wsp_8f2", "title": "x"})).unwrap_err();
        assert_eq!(
            error,
            Error::MissingField { fields: vec!["parent".into(), "state".into(), "checkout".into(), "harness".into()] }
        );
    }

    #[test]
    fn a_null_parent_is_present_not_missing() {
        let args = json!({"workspace": "wsp_8f2", "title": "x", "parent": null, "state": "pinned", "checkout": "root", "harness": "claude-code"});
        assert_eq!(parse_args::<ops::TaskCreate>(&args).unwrap().parent, None);
    }

    #[test]
    fn an_undeclared_field_is_rejected() {
        let error = parse_args::<ops::TaskFocus>(&json!({"task": "tsk_1", "force": true})).unwrap_err();
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
        assert_eq!(Stream::parse("pty:pty_2"), Some(Stream::Pty(PtyId("pty_2".into()))));
        assert_eq!(Stream::parse("tunnel:5173"), None);
    }

    #[test]
    fn errors_serialize_with_their_code() {
        let wire = serde_json::to_value(Error::PtyLocked { holder: "laptop".into() }).unwrap();
        assert_eq!(wire, json!({"code": "pty_locked", "holder": "laptop"}));
    }
}
