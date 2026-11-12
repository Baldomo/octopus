//! The harnesses a fresh server knows.

use crate::store::Harness;

/// Stands for the task's session id in a harness's start and resume args.
pub(crate) const SESSION: &str = "{session}";

pub(crate) fn names_session(args: &[String]) -> bool {
    args.iter().any(|arg| arg.contains(SESSION))
}

/// The harness a fresh store runs every agent with until the user sets it otherwise.
pub(crate) const CLAUDE_CODE: &str = "claude-code";

pub(crate) fn claude_code() -> Harness {
    Harness {
        argv: vec!["claude".into()],
        start: vec!["--session-id".into(), SESSION.into()],
        resume: vec!["--resume".into(), SESSION.into()],
        install: "npm install -g @anthropic-ai/claude-code".into(),
    }
}
