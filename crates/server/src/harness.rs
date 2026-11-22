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

/// The other harness a fresh store knows: OpenCode's TUI, which continues a session or starts it if absent.
pub(crate) const OPENCODE: &str = "opencode";

pub(crate) fn opencode() -> Harness {
    Harness {
        argv: vec!["opencode".into()],
        start: vec!["--session".into(), SESSION.into()],
        resume: vec!["--session".into(), SESSION.into()],
        install: "npm install -g @opencode/cli".into(),
    }
}

/// OpenCode's TUI settings, as JSON over its settings files.
const CLI_CONFIG: &str = "OPENCODE_CLI_CONFIG_CONTENT";

/// The env an agent runs in. OpenCode's tab strip is turned off, so its status follows only the task's own session
/// and not every session in the folder; the rest of any settings the user passes stands.
pub(crate) fn agent_env(harness: &str, mut env: Vec<(String, String)>) -> Vec<(String, String)> {
    if harness != OPENCODE {
        return env;
    }
    let given = env.iter().position(|(key, _)| key == CLI_CONFIG);
    let mut config = match given.and_then(|at| serde_json::from_str(&env[at].1).ok()) {
        Some(serde_json::Value::Object(config)) => config,
        _ => serde_json::Map::new(),
    };
    let tabs = config.entry("tabs").or_insert_with(|| serde_json::json!({}));
    if !tabs.is_object() {
        *tabs = serde_json::json!({});
    }
    tabs["mode"] = "off".into();
    let config = serde_json::Value::Object(config).to_string();
    match given {
        Some(at) => env[at].1 = config,
        None => env.push((CLI_CONFIG.to_string(), config)),
    }
    env
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cli_config(env: &[(String, String)]) -> Vec<serde_json::Value> {
        env.iter()
            .filter(|(k, _)| k == "OPENCODE_CLI_CONFIG_CONTENT")
            .map(|(_, v)| serde_json::from_str(v).expect("json"))
            .collect()
    }

    #[test]
    fn an_opencode_agent_runs_with_its_tab_strip_off() {
        assert_eq!(cli_config(&agent_env("opencode", vec![])), [serde_json::json!({"tabs": {"mode": "off"}})]);
    }

    #[test]
    fn an_opencode_agent_keeps_the_rest_of_the_user_s_cli_config() {
        let user = r#"{"theme":"x","tabs":{"scope":"global","mode":"on"}}"#;
        let env = vec![("OPENCODE_CLI_CONFIG_CONTENT".to_string(), user.to_string())];
        assert_eq!(
            cli_config(&agent_env("opencode", env)),
            [serde_json::json!({"theme": "x", "tabs": {"scope": "global", "mode": "off"}})]
        );
        let broken = vec![("OPENCODE_CLI_CONFIG_CONTENT".to_string(), "not json".to_string())];
        assert_eq!(cli_config(&agent_env("opencode", broken)), [serde_json::json!({"tabs": {"mode": "off"}})]);
    }

    #[test]
    fn other_agents_run_in_the_env_they_were_given() {
        let env = vec![("A".to_string(), "1".to_string())];
        assert_eq!(agent_env("claude-code", env.clone()), env);
    }
}
