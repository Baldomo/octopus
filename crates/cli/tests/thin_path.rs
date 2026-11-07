mod common;

use common::{
    SERVER, ServerProcess, agent, connect, create, echo_agent, eventually, focus, harness, holder_name, read_until,
    refused, send, server, set_harness, start, task, workspace, write_login_profile,
};
use octopus_client::Connection;
use octopus_proto::ops::{
    AgentReport, HarnessSet, TaskCreate, TaskFocus, TaskMove, TaskSetSession, TermClaim, TermClose, TermOpen,
    TermResize, TermSend, TermTakeLock,
};
use octopus_proto::types::{
    Agent, AgentStatus, CheckoutKind, ENV_SERVER, ENV_TASK, FRESH_SESSION, Hooks, SessionId, SessionStart, Stream,
    TaskId, TaskState,
};
use octopus_proto::{Error, Op, Outcome};
use octopus_server::{AGENT_RECLAIM_MS, now};
use serde_json::json;

fn locked(error: Error, holder: &str) {
    assert_eq!(error, Error::PtyLocked { holder: holder.into() });
}

#[tokio::test(flavor = "multi_thread")]
async fn a_focused_task_runs_its_agent_terminal_end_to_end() {
    let (dir, running) = server().await;
    let laptop = connect(&running, "laptop").await;
    let workspace = workspace(&laptop, dir.path()).await;
    set_harness(&laptop, "echo", echo_agent(&format!("agent:${ENV_TASK}@${ENV_SERVER}")), Hooks::ClaudeCode).await;

    let id = create(&laptop, &workspace, CheckoutKind::Root, "echo").await;
    assert_eq!(task(&laptop, &id).await.agent, Agent::NotStarted);
    focus(&laptop, &id).await;
    assert_eq!(task(&laptop, &id).await.agent, Agent::Running { status: AgentStatus::Idle });
    let agent = agent(&laptop, &id).await;
    laptop.call(&AgentReport { task: id.clone(), status: AgentStatus::Waiting }).await.expect("agent.report");
    assert_eq!(task(&laptop, &id).await.agent, Agent::Running { status: AgentStatus::Waiting });

    let mut live = laptop.open_data_stream(&agent).await.expect("attach");
    read_until(&mut live, &format!("agent:{id}@{SERVER}")).await;
    let claim = |cols, rows| TermClaim { task: id.clone(), panel: "agent".into(), cols, rows };
    laptop.call(&claim(80, 24)).await.expect("claim the free terminal");
    let write = TermSend { take_lock: false, ..send(&id, "agent", "ping") };
    laptop.call(&write).await.expect("term.send");
    read_until(&mut live, "ping").await;
    assert_eq!(holder_name(&task(&laptop, &id).await.panels[0]), Some("laptop"));

    let desk = connect(&running, "desk").await;
    let mut replay = desk.open_data_stream(&agent).await.expect("reattach");
    read_until(&mut replay, "ping").await;
    locked(refused(desk.call(&claim(90, 20)).await), "laptop");
    locked(refused(desk.call(&write).await), "laptop");
    let resize = TermResize { task: id.clone(), panel: "agent".into(), cols: 90, rows: 20 };
    locked(refused(desk.call(&resize).await), "laptop");
    desk.call(&TermTakeLock { task: id.clone(), panel: "agent".into(), cols: 100, rows: 30 }).await.expect("take");
    let panel = task(&laptop, &id).await.panels.remove(0);
    assert_eq!((holder_name(&panel), panel.cols, panel.rows), (Some("desk"), 100, 30));
    drop(replay);
    eventually("a holder that stops showing the terminal freeing its lock", async || {
        task(&laptop, &id).await.panels[0].lock_holder.is_none()
    })
    .await;

    let partial = desk.call_raw(TaskCreate::NAME, json!({ "workspace": workspace, "title": "x" })).await.expect("call");
    let missing = ["parent", "state", "checkout", "harness", "session"].map(String::from).to_vec();
    assert_eq!(partial, Outcome::Err(Error::MissingField { fields: missing }));

    laptop.call(&TaskMove { task: id.clone(), state: TaskState::Completed }).await.expect("task.move");
    eventually("the agent exiting after its task completed", async || !task(&laptop, &id).await.panels[0].running)
        .await;
    assert_eq!(task(&laptop, &id).await.agent, Agent::NotStarted);
    read_until(&mut desk.open_data_stream(&agent).await.expect("attach exited"), "ping").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_shell_panel_follows_the_agent_until_its_shell_exits_or_it_closes() {
    let (dir, running) = server().await;
    let laptop = connect(&running, "laptop").await;
    let workspace = workspace(&laptop, dir.path()).await;
    set_harness(&laptop, "echo", echo_agent("agent:up"), Hooks::None).await;
    let id = create(&laptop, &workspace, CheckoutKind::Root, "echo").await;
    focus(&laptop, &id).await;

    let shell = laptop.call(&TermOpen { task: id.clone(), directory: ".".into() }).await.expect("term.open").panel;
    let panels = task(&laptop, &id).await.panels;
    assert_eq!(panels.iter().map(|panel| &panel.id).collect::<Vec<_>>()[1..], [&shell]);
    assert_eq!(panels[0].name, "agent");
    let mut live = laptop.open_data_stream(&Stream::Pty(shell.clone())).await.expect("attach the shell");
    laptop.call(&send(&id, &shell.0, &format!("echo shell:${ENV_TASK}"))).await.expect("type into the shell");
    read_until(&mut live, &format!("shell:{id}")).await;
    let agent_closed = laptop.call(&TermClose { task: id.clone(), panel: "agent".into() }).await;
    assert!(matches!(refused(agent_closed), Error::Conflict { .. }));

    let closed = laptop.call(&TermOpen { task: id.clone(), directory: ".".into() }).await.expect("term.open").panel;
    let names: Vec<_> = task(&laptop, &id).await.panels.into_iter().map(|panel| panel.name).collect();
    assert_eq!(names[2], format!("{}-2", names[1]));
    laptop.call(&send(&id, &shell.0, "exit")).await.expect("exit the shell");
    eventually("the exited shell leaving the list", async || task(&laptop, &id).await.panels.len() == 2).await;
    laptop.call(&TermClose { task: id.clone(), panel: closed.0 }).await.expect("term.close");
    assert_eq!(task(&laptop, &id).await.panels.len(), 1);

    laptop.call(&TaskMove { task: id.clone(), state: TaskState::Completed }).await.expect("complete");
    let ended = laptop.call(&TermOpen { task: id, directory: ".".into() }).await;
    assert!(matches!(refused(ended), Error::Conflict { .. }));
}

/// The shell ignores the host's SIGTERM, so its exit lands only at the SIGKILL 5 s later, long after the close.
#[tokio::test(flavor = "multi_thread")]
async fn closing_a_shell_the_terminal_host_already_ended_drops_its_panel() {
    let (dir, running) = server().await;
    let laptop = connect(&running, "laptop").await;
    let workspace = workspace(&laptop, dir.path()).await;
    set_harness(&laptop, "echo", echo_agent("agent:up"), Hooks::None).await;
    let id = create(&laptop, &workspace, CheckoutKind::Root, "echo").await;
    let shell = laptop.call(&TermOpen { task: id.clone(), directory: ".".into() }).await.expect("term.open").panel;
    let mut live = laptop.open_data_stream(&Stream::Pty(shell.clone())).await.expect("attach the shell");
    laptop.call(&send(&id, &shell.0, "trap '' TERM; echo trap$((1+1))")).await.expect("ignore SIGTERM");
    read_until(&mut live, "trap2").await;

    let forget = json!({ "op": "forget", "id": shell });
    let (_, forgot) =
        octopus_proto::local::open_blocking(&dir.path().join("host.sock"), &forget).expect("reach the host");
    assert!(matches!(forgot, Outcome::Ok(_)), "{forgot:?}");

    let close = TermClose { task: id.clone(), panel: shell.0.clone() };
    laptop.call(&close).await.expect("term.close of a shell the host forgot");
    assert!(task(&laptop, &id).await.panels.iter().all(|panel| panel.id != shell));
    assert!(matches!(refused(laptop.call(&close).await), Error::NotFound { .. }));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_shell_panel_opens_before_its_login_profile_finishes_and_then_has_its_environment() {
    const PROFILE_SECONDS: u64 = 2;
    let dir = tempfile::tempdir().expect("tempdir");
    let home = dir.path().join("home");
    std::fs::create_dir_all(&home).expect("home");
    write_login_profile(&home, &format!("sleep {PROFILE_SECONDS}\nexport OCTOPUS_TEST_LOGIN=profile\n"));
    let server = ServerProcess::launched(dir.path(), &home).await;
    let laptop = server.connect("laptop").await;
    let workspace = workspace(&laptop, dir.path()).await;
    set_harness(&laptop, "echo", echo_agent("agent:up"), Hooks::None).await;
    let id = create(&laptop, &workspace, CheckoutKind::Root, "echo").await;

    let opening = std::time::Instant::now();
    let shell = laptop.call(&TermOpen { task: id.clone(), directory: ".".into() }).await.expect("term.open").panel;
    assert!(opening.elapsed().as_secs() < PROFILE_SECONDS, "term.open waited on the profile");
    let mut live = laptop.open_data_stream(&Stream::Pty(shell.clone())).await.expect("attach the shell");
    laptop.call(&send(&id, &shell.0, "echo login:$OCTOPUS_TEST_LOGIN")).await.expect("type into the shell");
    read_until(&mut live, "login:profile").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_missing_harness_runs_nothing_and_reports_its_install_command() {
    let (dir, running) = server().await;
    let laptop = connect(&running, "laptop").await;
    let workspace = workspace(&laptop, dir.path()).await;
    let install = "npm install -g @anthropic-ai/claude-code".to_string();
    let absent = harness("claude-code", vec!["octopus-test-no-such-binary".into()], Hooks::None);
    laptop.call(&HarnessSet { install: install.clone(), ..absent }).await.expect("harness.set");

    let missing = create(&laptop, &workspace, CheckoutKind::Root, "claude-code").await;
    focus(&laptop, &missing).await;
    let view = task(&laptop, &missing).await;
    assert_eq!(view.agent, Agent::HarnessNotInstalled { install });
    assert!(view.panels.is_empty());

    let unknown = create(&laptop, &workspace, CheckoutKind::Root, "aider").await;
    focus(&laptop, &unknown).await;
    assert_eq!(task(&laptop, &unknown).await.agent, Agent::HarnessUnknown);
    let report = refused(laptop.call(&AgentReport { task: missing, status: AgentStatus::Working }).await);
    assert!(matches!(&report, Error::Conflict { message } if message.contains("declares no hooks")), "{report:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_focuses_start_one_agent() {
    let (dir, running) = server().await;
    let starts = dir.path().join("starts");
    let laptop = connect(&running, "laptop").await;
    let desk = connect(&running, "desk").await;
    let workspace = workspace(&laptop, dir.path()).await;
    set_harness(&laptop, "echo", echo_agent(&format!("start >> {}", starts.display())), Hooks::None).await;
    let id = create(&laptop, &workspace, CheckoutKind::Root, "echo").await;

    let focus = TaskFocus { task: id.clone(), args: vec![] };
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
    set_harness(&laptop, "echo", echo_agent("agent:up"), Hooks::None).await;
    let id = create(&laptop, &workspace, CheckoutKind::Root, "echo").await;
    focus(&laptop, &id).await;
    laptop.call(&TermOpen { task: id.clone(), directory: ".".into() }).await.expect("term.open");
    let view = task(&laptop, &id).await;
    let agent = Stream::Pty(view.panels[0].id.clone());
    let mut live = laptop.open_data_stream(&agent).await.expect("attach");
    read_until(&mut live, "agent:up").await;
    laptop.call(&send(&id, "agent", "before-restart")).await.expect("term.send");
    read_until(&mut live, "before-restart").await;
    before.kill();

    let after = ServerProcess::run(dir.path()).await;
    let laptop = after.connect("laptop").await;
    let reattached = task(&laptop, &id).await;
    let ids = |panels: &[octopus_proto::types::Panel]| panels.iter().map(|panel| panel.id.clone()).collect::<Vec<_>>();
    assert_eq!(ids(&reattached.panels), ids(&view.panels));
    assert_eq!(reattached.agent, Agent::Running { status: AgentStatus::Idle });
    let mut replay = laptop.open_data_stream(&agent).await.expect("reattach");
    read_until(&mut replay, "before-restart").await;
    laptop.call(&send(&id, "agent", "after-restart")).await.expect("term.send after the restart");
    read_until(&mut replay, "after-restart").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn after_a_machine_restart_the_next_focus_starts_the_agent_again() {
    let dir = tempfile::tempdir().expect("tempdir");
    let before = start(dir.path(), "before.sock", "host.sock").await;
    let laptop = connect(&before, "laptop").await;
    let workspace = workspace(&laptop, dir.path()).await;
    set_harness(&laptop, "echo", echo_agent("agent:up"), Hooks::None).await;
    let id = create(&laptop, &workspace, CheckoutKind::Root, "echo").await;
    focus(&laptop, &id).await;

    let after = start(dir.path(), "after.sock", "rebooted-host.sock").await;
    let laptop = connect(&after, "laptop").await;
    let forgotten = task(&laptop, &id).await;
    assert_eq!((forgotten.agent, forgotten.panels), (Agent::NotStarted, vec![]));
    focus(&laptop, &id).await;
    assert_eq!(task(&laptop, &id).await.agent, Agent::Running { status: AgentStatus::Idle });
}

#[allow(unsafe_code, reason = "libSystem's responsibility call has no safe binding")]
#[tokio::test(flavor = "multi_thread")]
async fn an_agent_owns_its_terminal_and_on_macos_answers_to_the_terminal_host_for_privacy_permissions() {
    #[cfg(target_os = "macos")]
    unsafe extern "C" {
        fn responsibility_get_pid_responsible_for_pid(pid: i32) -> i32;
    }
    // Perl rather than a shell: bash takes its terminal as its controlling terminal on its own, which would hide a
    // spawn that leaves the PTY nobody's. tcgetpgrp answers only on the caller's controlling terminal.
    const PROBE: &str = r#"use POSIX; $| = 1;
        $SIG{WINCH} = sub { print "resized\n" };
        $SIG{INT} = sub { print "interrupted\n" };
        print "probe:$$:", (tcgetpgrp(0) == $$ ? 'foreground' : 'not foreground'), ":", getppid(), ":end\n";
        sleep 1 while 1;"#;
    let (dir, running) = server().await;
    let laptop = connect(&running, "laptop").await;
    let workspace = workspace(&laptop, dir.path()).await;
    set_harness(&laptop, "probe", vec!["perl".into(), "-e".into(), PROBE.into()], Hooks::None).await;
    let id = create(&laptop, &workspace, CheckoutKind::Root, "probe").await;
    focus(&laptop, &id).await;
    let mut live = laptop.open_data_stream(&agent(&laptop, &id).await).await.expect("attach");
    let seen = read_until(&mut live, ":end").await;
    let line = seen.split("probe:").nth(1).and_then(|rest| rest.split(":end").next()).expect("the probe's line");
    let pid: i32 = line.split(':').next().and_then(|pid| pid.parse().ok()).expect("a pid");
    let host: i32 = line.rsplit(':').next().and_then(|host| host.parse().ok()).expect("the host's pid");
    assert_eq!(line, format!("{pid}:foreground:{host}"), "the agent's controlling terminal is its PTY, which it leads");

    laptop.call(&interrupt(&id)).await.expect("ctrl-c");
    read_until(&mut live, "interrupted").await;
    laptop.call(&TermResize { task: id, panel: "agent".into(), cols: 120, rows: 40 }).await.expect("resize");
    read_until(&mut live, "resized").await;
    // SAFETY: the call reads only the kernel's record of a pid.
    #[cfg(target_os = "macos")]
    assert_eq!(unsafe { responsibility_get_pid_responsible_for_pid(pid) }, host);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_that_cannot_start_fails_the_focus_with_the_reason() {
    use std::os::unix::fs::PermissionsExt;

    let (dir, running) = server().await;
    let script = dir.path().join("script");
    std::fs::write(&script, "#!/no/such/interpreter\n").expect("write");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    let laptop = connect(&running, "laptop").await;
    let workspace = workspace(&laptop, dir.path()).await;
    set_harness(&laptop, "script", vec![script.to_string_lossy().into()], Hooks::None).await;
    let id = create(&laptop, &workspace, CheckoutKind::Root, "script").await;
    let message = format!("start {}: No such file or directory (os error 2)", script.display());
    assert_eq!(refused(laptop.call(&TaskFocus { task: id, args: vec![] }).await), Error::Internal { message });
}

#[tokio::test(flavor = "multi_thread")]
async fn an_agent_that_ignores_sigterm_and_sighup_is_killed_after_the_grace() {
    let (dir, running) = server().await;
    let laptop = connect(&running, "laptop").await;
    let workspace = workspace(&laptop, dir.path()).await;
    set_harness(&laptop, "stubborn", echo_agent("trapped; trap '' TERM HUP; echo ignoring"), Hooks::None).await;
    let id = create(&laptop, &workspace, CheckoutKind::Root, "stubborn").await;
    focus(&laptop, &id).await;
    read_until(&mut laptop.open_data_stream(&agent(&laptop, &id).await).await.expect("attach"), "ignoring").await;
    laptop.call(&TaskMove { task: id.clone(), state: TaskState::Completed }).await.expect("complete");
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    assert_eq!(task(&laptop, &id).await.agent, Agent::Running { status: AgentStatus::Idle }, "SIGTERM is ignored");
    eventually("the agent killed after the grace", async || task(&laptop, &id).await.agent == Agent::NotStarted).await;
}

async fn focus_prints(connection: &Connection, id: &TaskId, args: &[&str], line: &str) {
    let args = args.iter().map(ToString::to_string).collect();
    connection.call(&TaskFocus { task: id.clone(), args }).await.expect("focus");
    read_until(&mut connection.open_data_stream(&agent(connection, id).await).await.expect("attach"), line).await;
}

/// A harness whose agent prints the args after its argv, starting and resuming its session by id.
fn sessions_harness(id: &str, hooks: Hooks) -> HarnessSet {
    let [start, resume] =
        [["--session-id", "{session}"], ["--resume", "{session}"]].map(|args| args.map(String::from).to_vec());
    let argv = ["sh", "-c", "echo args: \"$@\"; exec cat", "sh"].map(String::from).to_vec();
    HarnessSet { start, resume, ..harness(id, argv, hooks) }
}

fn interrupt(id: &TaskId) -> TermSend {
    TermSend { enter: false, ..send(id, "agent", "\u{3}") }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_focus_s_args_reach_the_agent_it_starts_and_no_running_one() {
    let (dir, running) = server().await;
    let laptop = connect(&running, "laptop").await;
    let desk = connect(&running, "desk").await;
    let workspace = workspace(&laptop, dir.path()).await;
    laptop.call(&sessions_harness("sessions", Hooks::None)).await.expect("harness.set");
    let id = create(&laptop, &workspace, CheckoutKind::Root, "sessions").await;

    focus_prints(&laptop, &id, &["--model", "opus"], "args: --model opus --session-id ").await;
    let started = task(&laptop, &id).await;
    desk.call(&TaskFocus { task: id.clone(), args: vec!["--other".into()] }).await.expect("focus the running agent");
    assert_eq!(task(&desk, &id).await.panels, started.panels, "the running agent keeps the args it started with");

    laptop.call(&interrupt(&id)).await.expect("ctrl-c");
    eventually("the interrupted agent ending", async || task(&laptop, &id).await.agent == Agent::NotStarted).await;
    let session = started.session.id;
    focus_prints(&desk, &id, &["--other"], &format!("args: --other --resume {}", session.0)).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn every_start_after_a_session_s_first_resumes_it() {
    let (dir, running) = server().await;
    let laptop = connect(&running, "laptop").await;
    let workspace = workspace(&laptop, dir.path()).await;
    laptop.call(&sessions_harness("sessions", Hooks::None)).await.expect("harness.set");
    let unnamed = HarnessSet { start: vec![], ..sessions_harness("unnamed", Hooks::None) };
    assert!(
        matches!(refused(laptop.call(&unnamed).await), Error::InvalidArgs { .. }),
        "a resume its start never names"
    );

    let id = create(&laptop, &workspace, CheckoutKind::Root, "sessions").await;
    focus(&laptop, &id).await;
    let session = task(&laptop, &id).await.session.id;
    focus_prints(&laptop, &id, &[], &format!("args: --session-id {}", session.0)).await;

    laptop.call(&interrupt(&id)).await.expect("ctrl-c");
    eventually("the interrupted agent ending", async || task(&laptop, &id).await.agent == Agent::NotStarted).await;
    let ended = task(&laptop, &id).await;
    assert!(!ended.panels[0].running, "the ended agent's panel stays, showing its scrollback");
    focus_prints(&laptop, &id, &[], &format!("args: --resume {}", session.0)).await;

    laptop.call(&TaskMove { task: id.clone(), state: TaskState::Completed }).await.expect("complete");
    laptop.call(&TaskMove { task: id.clone(), state: TaskState::Pinned }).await.expect("reopen");
    assert_eq!(task(&laptop, &id).await.panels, vec![], "a reopened task's exited agent goes");
    focus_prints(&laptop, &id, &[], &format!("args: --resume {}", session.0)).await;

    for session in ["", FRESH_SESSION] {
        let set = TaskSetSession { task: id.clone(), session: SessionId(session.into()) };
        assert!(matches!(refused(laptop.call(&set).await), Error::InvalidArgs { .. }), "{session:?}");
    }
    let set = TaskSetSession { task: id.clone(), session: SessionId("elsewhere".into()) };
    laptop.call(&set).await.expect("task.set_session");
    assert_eq!(task(&laptop, &id).await.panels, vec![]);
    focus_prints(&laptop, &id, &[], "args: --resume elsewhere").await;

    let imported = TaskCreate {
        workspace,
        title: "imported".into(),
        parent: None,
        state: TaskState::Pinned,
        checkout: CheckoutKind::Root,
        harness: "sessions".into(),
        session: SessionStart::Resume(SessionId("demo-session".into())),
    };
    let imported = laptop.call(&imported).await.expect("task.create").task;
    focus_prints(&laptop, &imported, &[], "args: --resume demo-session").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_hooked_agent_resumes_its_session_only_once_a_prompt_created_it() {
    let (dir, running) = server().await;
    let laptop = connect(&running, "laptop").await;
    let workspace = workspace(&laptop, dir.path()).await;
    laptop.call(&sessions_harness("hooked", Hooks::ClaudeCode)).await.expect("harness.set");
    let id = create(&laptop, &workspace, CheckoutKind::Root, "hooked").await;
    let session = task(&laptop, &id).await.session.id;

    focus_prints(&laptop, &id, &[], &format!("--session-id {}", session.0)).await;
    laptop.call(&interrupt(&id)).await.expect("ctrl-c");
    eventually("the unprompted agent ending", async || task(&laptop, &id).await.agent == Agent::NotStarted).await;
    assert!(!task(&laptop, &id).await.session.started);
    focus_prints(&laptop, &id, &[], &format!("--session-id {}", session.0)).await;

    laptop.call(&AgentReport { task: id.clone(), status: AgentStatus::Working }).await.expect("agent.report");
    assert!(task(&laptop, &id).await.session.started, "a prompt creates the session");
    laptop.call(&interrupt(&id)).await.expect("ctrl-c");
    eventually("the prompted agent ending", async || task(&laptop, &id).await.agent == Agent::NotStarted).await;
    focus_prints(&laptop, &id, &[], &format!("--resume {}", session.0)).await;
}

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
    set_harness(&laptop, "login", vec!["octopus-test-agent".into()], Hooks::None).await;
    let id = create(&laptop, &workspace, CheckoutKind::Root, "login").await;

    focus(&laptop, &id).await;
    assert_eq!(task(&laptop, &id).await.agent, Agent::HarnessNotInstalled { install: "true".into() });

    write_login_profile(
        &home,
        &format!("export PATH=\"{}:$PATH\"\nexport OCTOPUS_TEST_LOGIN=profile\n", bin.display()),
    );
    focus_prints(&laptop, &id, &[], "login:profile").await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_server_stops_an_agent_idle_silent_and_unshown_for_the_reclaim_time() {
    let (dir, running) = server().await;
    let laptop = connect(&running, "laptop").await;
    let workspace = workspace(&laptop, dir.path()).await;
    let sh = |script: &str| ["sh", "-c", script].map(String::from).to_vec();
    set_harness(&laptop, "agent", sh("sleep 600 & wait"), Hooks::ClaudeCode).await;
    set_harness(&laptop, "detaching", sh("set -m; sleep 60 & wait"), Hooks::None).await;
    let before = now();
    let mut tasks = Vec::new();
    for harness in ["agent", "agent", "agent", "detaching"] {
        let id = create(&laptop, &workspace, CheckoutKind::Root, harness).await;
        focus(&laptop, &id).await;
        tasks.push(id);
    }
    let [idle, waiting, shown, detached] = <[TaskId; 4]>::try_from(tasks).expect("four tasks");
    laptop.call(&AgentReport { task: waiting.clone(), status: AgentStatus::Waiting }).await.expect("agent.report");
    let _showing = laptop.open_data_stream(&agent(&laptop, &shown).await).await.expect("show");
    let after = now();

    assert_eq!(running.server.reclaim_agents(before + AGENT_RECLAIM_MS - 1), vec![]);
    let due = running.server.reclaim_agents(after + AGENT_RECLAIM_MS);
    assert_eq!(due, vec![idle.clone()], "a waiting, a shown and a detaching agent keep running");
    eventually("the reclaimed agent ending", async || task(&laptop, &idle).await.agent == Agent::NotStarted).await;
    assert!(!task(&laptop, &idle).await.panels[0].running, "a reclaimed agent ends like any other");
    for kept in [&waiting, &shown, &detached] {
        assert!(matches!(task(&laptop, kept).await.agent, Agent::Running { .. }));
    }
    focus(&laptop, &idle).await;
    assert_eq!(task(&laptop, &idle).await.agent, Agent::Running { status: AgentStatus::Idle });
}
