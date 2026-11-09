mod common;

use std::process::Command;

use common::{OCTOPUS, connect, server};
use octopus_proto::ops::{TaskCreate, commands};
use octopus_proto::{Error, Op, Outcome};
use serde_json::json;

#[test]
fn a_call_missing_flags_names_all_of_them_at_once() {
    let output = Command::new(OCTOPUS)
        .args(["task", "create", "--server", "mac-mini", "--workspace", "octopus", "--title", "x"])
        .output()
        .expect("run octopus");
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "error: missing required flags: --parent (pass none for a root task), --state, --checkout, --harness, --session (pass fresh for a fresh session)\n"
    );
}

#[test]
fn a_home_other_than_the_accounts_registers_no_service() {
    let home = tempfile::tempdir().expect("tempdir");
    let output = Command::new(OCTOPUS)
        .env("HOME", home.path())
        .args(["server", "uninstall", "--name", "foreign-home"])
        .output()
        .expect("run octopus");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    let account = nix::unistd::User::from_uid(nix::unistd::getuid()).expect("look up").expect("an account");
    let named = |dir: &std::path::Path| stderr.contains(&dir.display().to_string());
    assert!(named(home.path()) && named(&account.dir), "{stderr}");
}

#[tokio::test(flavor = "multi_thread")]
async fn every_command_sends_an_op_the_server_serves_and_a_call_names_every_missing_field() {
    let (_dir, running) = server().await;
    let laptop = connect(&running.socket, "laptop").await;
    for command in commands() {
        let outcome = laptop.call_raw(command.op, json!({})).await.expect("call");
        assert!(
            !matches!(outcome, Outcome::Err(Error::UnknownOp { .. })),
            "the server does not serve {}, which the CLI sends",
            command.op
        );
    }
    let partial = laptop.call_raw(TaskCreate::NAME, json!({ "workspace": "wsp_1", "title": "x" })).await;
    let missing = ["parent", "state", "checkout", "harness", "session"].map(String::from).to_vec();
    assert_eq!(partial.expect("call"), Outcome::Err(Error::MissingField { fields: missing }));
}
