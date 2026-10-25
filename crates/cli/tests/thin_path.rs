mod common;

use common::{
    SERVER, ServerProcess, connect, create, echo_agent, eventually, read_until, shell_var, start, task, workspace,
};
use octopus_client::{CallError, Connection};
use octopus_proto::ops::{AgentReport, HarnessSet, TaskCreate, TaskFocus, TaskMove, TaskSetSession, TermSend};
use octopus_proto::types::{Agent, AgentStatus, Checkout, ENV_SERVER, ENV_TASK, SessionId, Stream, TaskId, TaskState};
use octopus_proto::{Error, Op, Outcome};
use serde_json::json;

async fn set_harness(connection: &Connection, id: &str, argv: Vec<String>, hooks: bool) {
    let harness = HarnessSet { id: id.into(), argv, install: "true".into(), start: vec![], resume: vec![], hooks };
    connection.call(&harness).await.expect("harness.set");
}

fn send(task: &TaskId, text: &str) -> TermSend {
    TermSend { task: task.clone(), panel: "agent".into(), text: text.into(), enter: true, take_lock: false }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_focused_task_runs_its_agent_terminal_end_to_end() {
    let dir = tempfile::tempdir().expect("tempdir");
    let running = start(dir.path(), "server.sock", "host.sock");
    let laptop = connect(&running, "laptop").await;
    let workspace = workspace(&laptop, dir.path()).await;
    set_harness(&laptop, "echo", echo_agent(&format!("agent:{}@{}", shell_var(ENV_TASK), shell_var(ENV_SERVER))), true)
        .await;

    let id = create(&laptop, &workspace, Checkout::Root, "echo").await;
    assert_eq!(task(&laptop, &id).await.agent, Agent::NotStarted);
    laptop.call(&TaskFocus { task: id.clone() }).await.expect("task.focus");
    let view = task(&laptop, &id).await;
    assert_eq!(view.agent, Agent::Running { status: AgentStatus::Idle });
    let agent = Stream::Pty(view.panels.first().expect("agent panel").id.clone());
    laptop.call(&AgentReport { task: id.clone(), status: AgentStatus::Waiting }).await.expect("agent.report");
    assert_eq!(task(&laptop, &id).await.agent, Agent::Running { status: AgentStatus::Waiting });

    let mut live = laptop.open_data_stream(&agent).await.expect("attach");
    read_until(&mut live, &format!("agent:{id}@{SERVER}")).await;
    laptop.call(&send(&id, "ping")).await.expect("term.send");
    read_until(&mut live, "ping").await;
    assert_eq!(task(&laptop, &id).await.panels[0].lock_holder.as_deref(), Some("laptop"));

    let desk = connect(&running, "desk").await;
    let mut replay = desk.open_data_stream(&agent).await.expect("reattach");
    read_until(&mut replay, "ping").await;
    match desk.call(&send(&id, "ping")).await {
        Err(CallError::Server(error)) => assert_eq!(error, Error::PtyLocked { holder: "laptop".into() }),
        other => panic!("expected pty_locked, got {other:?}"),
    }

    let partial = desk.call_raw(TaskCreate::NAME, json!({ "workspace": workspace, "title": "x" })).await.expect("call");
    let missing = ["parent", "state", "checkout", "harness", "session"].map(String::from).to_vec();
    assert_eq!(partial, Outcome::Err(Error::MissingField { fields: missing }));

    laptop.call(&TaskMove { task: id.clone(), state: TaskState::Completed }).await.expect("task.move");
    eventually("the agent exiting after its task completed", async || task(&laptop, &id).await.agent == Agent::Exited)
        .await;
    read_until(&mut desk.open_data_stream(&agent).await.expect("attach exited"), "ping").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_missing_harness_runs_nothing_and_reports_its_install_command() {
    let dir = tempfile::tempdir().expect("tempdir");
    let running = start(dir.path(), "server.sock", "host.sock");
    let laptop = connect(&running, "laptop").await;
    let workspace = workspace(&laptop, dir.path()).await;
    let install = "npm install -g @anthropic-ai/claude-code".to_string();
    let argv = vec!["octopus-test-no-such-binary".into()];
    let harness = HarnessSet {
        id: "claude-code".into(),
        argv,
        install: install.clone(),
        start: vec![],
        resume: vec![],
        hooks: false,
    };
    laptop.call(&harness).await.expect("harness.set");

    let missing = create(&laptop, &workspace, Checkout::Root, "claude-code").await;
    laptop.call(&TaskFocus { task: missing.clone() }).await.expect("focus");
    let view = task(&laptop, &missing).await;
    assert_eq!((view.agent, view.focused_at), (Agent::HarnessNotInstalled { install }, None));
    assert!(view.panels.is_empty());

    let unknown = create(&laptop, &workspace, Checkout::Root, "aider").await;
    laptop.call(&TaskFocus { task: unknown.clone() }).await.expect("focus");
    assert_eq!(task(&laptop, &unknown).await.agent, Agent::HarnessUnknown);
    match laptop.call(&AgentReport { task: missing, status: AgentStatus::Working }).await {
        Err(CallError::Server(Error::Conflict { message })) => {
            assert!(message.contains("declares no hooks"), "{message}")
        }
        other => panic!("a harness without hooks must refuse reports, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_focuses_start_one_agent() {
    let dir = tempfile::tempdir().expect("tempdir");
    let starts = dir.path().join("starts");
    let running = start(dir.path(), "server.sock", "host.sock");
    let laptop = connect(&running, "laptop").await;
    let desk = connect(&running, "desk").await;
    let workspace = workspace(&laptop, dir.path()).await;
    set_harness(&laptop, "echo", echo_agent(&format!("start >> {}", starts.display())), false).await;
    let id = create(&laptop, &workspace, Checkout::Root, "echo").await;

    let focus = TaskFocus { task: id.clone() };
    let (first, second) = tokio::join!(laptop.call(&focus), desk.call(&focus));
    first.expect("focus from laptop");
    second.expect("focus from desk");
    eventually("the agent writing its start line", async || starts.exists()).await;
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert_eq!(std::fs::read_to_string(&starts).expect("read starts").lines().count(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_outlives_a_restart_of_its_server_process() {
    let dir = tempfile::tempdir().expect("tempdir");
    let before = ServerProcess::run(dir.path()).await;
    let laptop = before.connect("laptop").await;
    let workspace = workspace(&laptop, dir.path()).await;
    set_harness(&laptop, "echo", echo_agent("agent:up"), false).await;
    let id = create(&laptop, &workspace, Checkout::Root, "echo").await;
    laptop.call(&TaskFocus { task: id.clone() }).await.expect("focus");
    let view = task(&laptop, &id).await;
    let agent = Stream::Pty(view.panels[0].id.clone());
    let mut live = laptop.open_data_stream(&agent).await.expect("attach");
    read_until(&mut live, "agent:up").await;
    laptop.call(&send(&id, "before-restart")).await.expect("term.send");
    read_until(&mut live, "before-restart").await;
    before.kill();

    let after = ServerProcess::run(dir.path()).await;
    let laptop = after.connect("laptop").await;
    let reattached = task(&laptop, &id).await;
    assert_eq!((reattached.focused_at, &reattached.panels[0].id), (view.focused_at, &view.panels[0].id));
    assert_eq!(reattached.agent, Agent::Running { status: AgentStatus::Idle });
    let mut replay = laptop.open_data_stream(&agent).await.expect("reattach");
    read_until(&mut replay, "before-restart").await;
    laptop.call(&send(&id, "after-restart")).await.expect("term.send after the restart");
    read_until(&mut replay, "after-restart").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn after_a_machine_restart_the_next_focus_starts_the_agent_again() {
    let dir = tempfile::tempdir().expect("tempdir");
    let before = start(dir.path(), "before.sock", "host.sock");
    let laptop = connect(&before, "laptop").await;
    let workspace = workspace(&laptop, dir.path()).await;
    set_harness(&laptop, "echo", echo_agent("agent:up"), false).await;
    let id = create(&laptop, &workspace, Checkout::Root, "echo").await;
    laptop.call(&TaskFocus { task: id.clone() }).await.expect("focus");
    assert!(task(&laptop, &id).await.focused_at.is_some());

    let after = start(dir.path(), "after.sock", "rebooted-host.sock");
    let laptop = connect(&after, "laptop").await;
    let forgotten = task(&laptop, &id).await;
    assert_eq!((forgotten.focused_at, forgotten.agent, forgotten.panels), (None, Agent::NotStarted, vec![]));
    laptop.call(&TaskFocus { task: id.clone() }).await.expect("focus after restart");
    assert_eq!(task(&laptop, &id).await.agent, Agent::Running { status: AgentStatus::Idle });
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code, reason = "libSystem's responsibility call has no safe binding")]
#[tokio::test(flavor = "multi_thread")]
async fn an_agent_owns_its_terminal_and_answers_for_its_own_privacy_permissions() {
    use octopus_proto::ops::TermResize;

    unsafe extern "C" {
        fn responsibility_get_pid_responsible_for_pid(pid: i32) -> i32;
    }
    // Perl rather than a shell: bash takes its terminal as its controlling terminal on its own, which would hide a
    // spawn that leaves the PTY nobody's. tcgetpgrp answers only on the caller's controlling terminal.
    const PROBE: &str = r#"use POSIX; $| = 1;
        $SIG{WINCH} = sub { print "resized\n" };
        $SIG{INT} = sub { print "interrupted\n" };
        print "probe:$$:", (tcgetpgrp(0) == $$ ? 'foreground' : 'not foreground'), ":end\n";
        sleep 1 while 1;"#;
    let dir = tempfile::tempdir().expect("tempdir");
    let running = start(dir.path(), "server.sock", "host.sock");
    let laptop = connect(&running, "laptop").await;
    let workspace = workspace(&laptop, dir.path()).await;
    set_harness(&laptop, "probe", vec!["perl".into(), "-e".into(), PROBE.into()], false).await;
    let id = create(&laptop, &workspace, Checkout::Root, "probe").await;
    laptop.call(&TaskFocus { task: id.clone() }).await.expect("focus");
    let agent = Stream::Pty(task(&laptop, &id).await.panels[0].id.clone());
    let mut live = laptop.open_data_stream(&agent).await.expect("attach");
    let seen = read_until(&mut live, ":end").await;
    let line = seen.split("probe:").nth(1).and_then(|rest| rest.split(":end").next()).expect("the probe's line");
    let pid: i32 = line.split(':').next().and_then(|pid| pid.parse().ok()).expect("a pid");
    assert_eq!(line, format!("{pid}:foreground"), "the agent's controlling terminal is its PTY, which it leads");

    let interrupt =
        TermSend { task: id.clone(), panel: "agent".into(), text: "\u{3}".into(), enter: false, take_lock: true };
    laptop.call(&interrupt).await.expect("ctrl-c");
    read_until(&mut live, "interrupted").await;
    laptop.call(&TermResize { task: id, panel: "agent".into(), cols: 120, rows: 40 }).await.expect("resize");
    read_until(&mut live, "resized").await;
    // SAFETY: the call reads only the kernel's record of a pid.
    assert_eq!(unsafe { responsibility_get_pid_responsible_for_pid(pid) }, pid);
}

#[cfg(target_os = "macos")]
#[tokio::test(flavor = "multi_thread")]
async fn an_agent_that_cannot_start_fails_the_focus_with_the_reason() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().expect("tempdir");
    let garbage = dir.path().join("garbage");
    std::fs::write(&garbage, [0u8, 1, 2]).expect("write");
    std::fs::set_permissions(&garbage, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    let running = start(dir.path(), "server.sock", "host.sock");
    let laptop = connect(&running, "laptop").await;
    let workspace = workspace(&laptop, dir.path()).await;
    set_harness(&laptop, "garbage", vec![garbage.to_string_lossy().into()], false).await;
    let id = create(&laptop, &workspace, Checkout::Root, "garbage").await;
    match laptop.call(&TaskFocus { task: id }).await {
        Err(CallError::Server(Error::Internal { message })) => {
            assert_eq!(message, format!("start {}: Exec format error (os error 8)", garbage.display()));
        }
        other => panic!("a focus whose agent cannot start fails, got {other:?}"),
    }
}

/// Unix only: Windows has no SIGTERM for an agent to ignore.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn an_agent_that_ignores_sigterm_and_sighup_is_killed_after_the_grace() {
    let dir = tempfile::tempdir().expect("tempdir");
    let running = start(dir.path(), "server.sock", "host.sock");
    let laptop = connect(&running, "laptop").await;
    let workspace = workspace(&laptop, dir.path()).await;
    set_harness(&laptop, "stubborn", echo_agent("trapped; trap '' TERM HUP; echo ignoring"), false).await;
    let id = create(&laptop, &workspace, Checkout::Root, "stubborn").await;
    laptop.call(&TaskFocus { task: id.clone() }).await.expect("focus");
    let agent = Stream::Pty(task(&laptop, &id).await.panels[0].id.clone());
    read_until(&mut laptop.open_data_stream(&agent).await.expect("attach"), "ignoring").await;
    laptop.call(&TaskMove { task: id.clone(), state: TaskState::Completed }).await.expect("complete");
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    assert_eq!(task(&laptop, &id).await.agent, Agent::Running { status: AgentStatus::Idle }, "SIGTERM is ignored");
    eventually("the agent killed after the grace", async || task(&laptop, &id).await.agent == Agent::Exited).await;
}

/// Prints the agent's arguments after `args:`, then stays open.
fn args_agent() -> Vec<String> {
    if cfg!(windows) {
        ["cmd", "/q", "/k", "echo args:"].map(String::from).to_vec()
    } else {
        ["sh", "-c", "echo args: \"$@\"; exec cat", "sh"].map(String::from).to_vec()
    }
}

async fn focus_prints(connection: &Connection, id: &TaskId, line: &str) {
    connection.call(&TaskFocus { task: id.clone() }).await.expect("focus");
    let agent = Stream::Pty(task(connection, id).await.panels[0].id.clone());
    read_until(&mut connection.open_data_stream(&agent).await.expect("attach"), line).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn every_start_after_a_session_s_first_resumes_it() {
    let dir = tempfile::tempdir().expect("tempdir");
    let running = start(dir.path(), "server.sock", "host.sock");
    let laptop = connect(&running, "laptop").await;
    let workspace = workspace(&laptop, dir.path()).await;
    let [start, resume] =
        [["--session-id", "{session}"], ["--resume", "{session}"]].map(|args| args.map(String::from).to_vec());
    let harness =
        HarnessSet { id: "sessions".into(), argv: args_agent(), start, resume, install: "true".into(), hooks: false };
    laptop.call(&harness).await.expect("harness.set");

    let id = create(&laptop, &workspace, Checkout::Root, "sessions").await;
    laptop.call(&TaskFocus { task: id.clone() }).await.expect("focus");
    let session = task(&laptop, &id).await.session.expect("the first start names a session");
    focus_prints(&laptop, &id, &format!("args: --session-id {}", session.0)).await;

    laptop.call(&TaskMove { task: id.clone(), state: TaskState::Completed }).await.expect("complete");
    laptop.call(&TaskMove { task: id.clone(), state: TaskState::Pinned }).await.expect("reopen");
    assert_eq!(task(&laptop, &id).await.panels, vec![], "a reopened task's exited agent goes");
    focus_prints(&laptop, &id, &format!("args: --resume {}", session.0)).await;

    let empty = TaskSetSession { task: id.clone(), session: SessionId(String::new()) };
    assert!(matches!(laptop.call(&empty).await, Err(CallError::Server(Error::InvalidArgs { .. }))));
    let set = TaskSetSession { task: id.clone(), session: SessionId("elsewhere".into()) };
    laptop.call(&set).await.expect("task.set_session");
    let pointed = task(&laptop, &id).await;
    assert_eq!((pointed.focused_at, pointed.panels), (None, vec![]));
    focus_prints(&laptop, &id, "args: --resume elsewhere").await;

    let imported = TaskCreate {
        workspace,
        title: "imported".into(),
        parent: None,
        state: TaskState::Pinned,
        checkout: Checkout::Root,
        harness: "sessions".into(),
        session: Some(SessionId("demo-session".into())),
    };
    let imported = laptop.call(&imported).await.expect("task.create").task;
    focus_prints(&laptop, &imported, "args: --resume demo-session").await;
}

/// Unix only: a Windows logon task starts the server in the account's whole environment.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_launched_server_finds_and_runs_agents_in_the_login_environment() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().expect("tempdir");
    let home = dir.path().join("home");
    let bin = home.join("bin");
    std::fs::create_dir_all(&bin).expect("bin");
    let agent = bin.join("octopus-test-agent");
    std::fs::write(&agent, "#!/bin/sh\necho \"login:$OCTOPUS_TEST_LOGIN\"\nexec cat\n").expect("agent");
    std::fs::set_permissions(&agent, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    let server = ServerProcess::launched(dir.path(), &home).await;
    let laptop = server.connect("laptop").await;
    let workspace = workspace(&laptop, dir.path()).await;
    set_harness(&laptop, "login", vec!["octopus-test-agent".into()], false).await;
    let id = create(&laptop, &workspace, Checkout::Root, "login").await;

    laptop.call(&TaskFocus { task: id.clone() }).await.expect("focus");
    assert_eq!(task(&laptop, &id).await.agent, Agent::HarnessNotInstalled { install: "true".into() });

    let profile = format!("export PATH=\"{}:$PATH\"\nexport OCTOPUS_TEST_LOGIN=profile\n", bin.display());
    for file in [".profile", ".zprofile"] {
        std::fs::write(home.join(file), &profile).expect("profile");
    }
    focus_prints(&laptop, &id, "login:profile").await;
}
