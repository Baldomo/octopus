mod common;

use std::process::Command;

use common::{OCTOPUS, connect, start};
use octopus_proto::ops::commands;
use octopus_proto::{Error, Outcome};
use serde_json::json;

#[test]
fn a_call_missing_flags_names_all_of_them_at_once() {
    let output = Command::new(OCTOPUS)
        .args(["task", "create", "--server", "mac-mini", "--workspace", "octopus", "--title", "x"])
        .output()
        .expect("run octopus");
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "error: missing required flags: --parent (pass none for a root task), --state, --checkout, --harness, --session (pass none for a fresh session)\n"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn every_command_sends_an_op_the_server_serves() {
    let dir = tempfile::tempdir().expect("tempdir");
    let running = start(dir.path(), "server.sock", "host.sock");
    let laptop = connect(&running, "laptop").await;
    for command in commands() {
        let outcome = laptop.call_raw(command.op, json!({})).await.expect("call");
        assert!(
            !matches!(outcome, Outcome::Err(Error::UnknownOp { .. })),
            "the server does not serve {}, which the CLI sends",
            command.op
        );
    }
}
