//! The harnesses a fresh server knows, and the hooks the server installs in a harness's agent, through which the
//! agent reports its status.

use std::collections::BTreeMap;
use std::path::Path;

use octopus_proto::types::{AgentStatus, ENV_SERVER, ENV_TASK, Hooks, to_wire};
use serde_json::{Value, json};

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
        hooks: Hooks::ClaudeCode,
    }
}

/// The tools that put a question to the user rather than act.
macro_rules! asking_tools {
    () => {
        "AskUserQuestion|ExitPlanMode"
    };
}

/// Each Claude Code event the agent reports on, the matcher its hook fires for, and the status it reports. The
/// matchers of one event exclude each other, since Claude Code runs every matching hook at once.
const CLAUDE_CODE_EVENTS: &[(&str, &str, AgentStatus)] = &[
    ("UserPromptSubmit", "", AgentStatus::Working),
    ("PreToolUse", concat!("^(?!(", asking_tools!(), ")$)"), AgentStatus::Working),
    ("PreToolUse", asking_tools!(), AgentStatus::Waiting),
    ("PostToolUse", "", AgentStatus::Working),
    ("PermissionRequest", "", AgentStatus::Waiting),
    ("Notification", "permission_prompt|elicitation_dialog|elicitation_url_dialog", AgentStatus::Waiting),
    ("Stop", "", AgentStatus::Idle),
    ("StopFailure", "", AgentStatus::Idle),
];

/// How long Claude Code waits on a report before it goes on without it.
const REPORT_TIMEOUT_SECONDS: u64 = 10;

/// The args after argv that install `hooks` in the agent, each hook reporting through `octopus`, the server's own
/// executable.
pub fn args(hooks: Hooks, octopus: &Path) -> Vec<String> {
    match hooks {
        Hooks::None => vec![],
        Hooks::ClaudeCode => vec!["--settings".into(), claude_code_settings(octopus).to_string()],
    }
}

/// Claude Code reads a hook's stdout as context or a decision, and its exit status 2 as a block, so a report prints
/// nothing and a failed one exits 1, which Claude Code only notes.
fn claude_code_settings(octopus: &Path) -> Value {
    let mut events: BTreeMap<&str, Vec<Value>> = BTreeMap::new();
    for (event, matcher, status) in CLAUDE_CODE_EVENTS {
        let command = format!(
            "{} agent report --server \"${ENV_SERVER}\" --task \"${ENV_TASK}\" --status {} >/dev/null || exit 1",
            quoted(octopus),
            to_wire(status)
        );
        let hook = json!({ "type": "command", "command": command, "timeout": REPORT_TIMEOUT_SECONDS });
        events.entry(event).or_default().push(json!({ "matcher": matcher, "hooks": [hook] }));
    }
    json!({ "hooks": events })
}

/// `path` as one word of a POSIX shell command.
fn quoted(path: &Path) -> String {
    format!("'{}'", path.to_string_lossy().replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn claude_code_hooks_report_each_event_as_its_status_to_the_agents_own_task() {
        let args = args(Hooks::ClaudeCode, Path::new("/opt/it's/octopus"));
        let [flag, settings] = args.as_slice() else { panic!("one flag and its settings, got {args:?}") };
        assert_eq!(flag, "--settings");
        let settings: Value = serde_json::from_str(settings).expect("the settings are JSON");
        let stop = &settings["hooks"]["Stop"][0]["hooks"][0]["command"];
        let report = r#"'/opt/it'\''s/octopus' agent report --server "$OCTOPUS_SERVER" --task "$OCTOPUS_TASK""#;
        assert_eq!(stop, &format!("{report} --status idle >/dev/null || exit 1"));
        let asking = &settings["hooks"]["PreToolUse"];
        assert_eq!(
            (&asking[0]["matcher"], &asking[1]["matcher"]),
            (&json!("^(?!(AskUserQuestion|ExitPlanMode)$)"), &json!("AskUserQuestion|ExitPlanMode"))
        );
        assert_eq!(super::args(Hooks::None, Path::new("/opt/octopus")), Vec::<String>::new());
    }
}
