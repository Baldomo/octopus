//! Each task's agent: which harness runs it, when it starts, the status it reports and when it is reclaimed.

use std::path::PathBuf;
use std::sync::Arc;

use octopus_proto::Error;
use octopus_proto::Event;
use octopus_proto::ops::{AgentReport, HarnessSet, Revised, TaskFocus};
use octopus_proto::types::{AGENT_PANEL, AgentStatus, Hooks, PtyId, TaskId};

use crate::store::{Harness, PanelRow, TaskRow};
use crate::terminals::Terminal;
use crate::{Server, State, harness, lock, login, now, report};

/// How long an agent stays idle, silent and unshown before the server stops it to reclaim its memory.
pub const AGENT_RECLAIM_MS: i64 = 30 * 60 * 1000;

pub(crate) enum AgentSlot {
    /// The agent's terminal, running or ended, with the status the agent last reported and when it took it.
    Spawned {
        terminal: Arc<Terminal>,
        status: AgentStatus,
        since: i64,
    },
    HarnessNotInstalled {
        install: String,
    },
    HarnessUnknown,
}

impl AgentSlot {
    pub(crate) fn terminal(&self) -> Option<&Arc<Terminal>> {
        match self {
            Self::Spawned { terminal, .. } => Some(terminal),
            _ => None,
        }
    }
}

impl State {
    /// Ends the task's agent terminal and drops its scrollback.
    pub(crate) fn forget_agent(&mut self, task: &TaskId) {
        if let Some(AgentSlot::Spawned { terminal, .. }) = self.agents.remove(task)
            && let Err(error) = terminal.forget()
        {
            report!("forget the agent terminal of {task}: {error}");
        }
    }

    fn agent_running(&self, task: &TaskId) -> bool {
        self.agent_terminal(task).is_some_and(|terminal| terminal.running())
    }

    /// The agent's terminal when the agent runs and has shown idle, with every terminal of its task unshown, for
    /// `AGENT_RECLAIM_MS` at `now`.
    fn idle_unshown_agent(&self, task: &TaskId, now: i64) -> Option<Arc<Terminal>> {
        let Some(AgentSlot::Spawned { terminal, status: AgentStatus::Idle, since }) = self.agents.get(task) else {
            return None;
        };
        let unshown = self.terminals(task).map(|(_, terminal)| terminal.unshown_since()).collect::<Option<Vec<_>>>()?;
        let quiet_since = unshown.into_iter().fold(*since, i64::max);
        (terminal.running() && now - quiet_since >= AGENT_RECLAIM_MS).then(|| Arc::clone(terminal))
    }

    pub(crate) fn agent_terminal(&self, task: &TaskId) -> Option<&Arc<Terminal>> {
        self.agents.get(task).and_then(AgentSlot::terminal)
    }
}

impl Server {
    pub(crate) fn harness_set(&self, op: HarnessSet) -> Result<Revised, Error> {
        if op.argv.is_empty() {
            return Err(Error::invalid("argv is empty"));
        }
        if harness::names_session(&op.resume) && !harness::names_session(&op.start) {
            return Err(Error::invalid(format!("resume names {} and start does not", harness::SESSION)));
        }
        let harness =
            Harness { argv: op.argv, start: op.start, resume: op.resume, install: op.install, hooks: op.hooks };
        let mut state = lock(&self.state);
        let rev = state.store.set_harness(&op.id, &harness)?;
        self.publish(&state, rev, Event::HarnessUpdated { harness: op.id });
        Ok(Revised { rev })
    }

    pub(crate) fn agent_report(&self, op: AgentReport) -> Result<Revised, Error> {
        let mut state = lock(&self.state);
        let mut row = state.store.task(&op.task)?;
        if state.store.harness(&row.harness)?.is_none_or(|harness| harness.hooks == Hooks::None) {
            return Err(Error::conflict(format!("harness {} declares no hooks", row.harness)));
        }
        match state.agents.get_mut(&row.id) {
            Some(AgentSlot::Spawned { terminal, status, since }) if terminal.running() => {
                if *status != op.status {
                    (*status, *since) = (op.status, now());
                }
            }
            _ => return Err(Error::conflict(format!("the agent of {} is not running", row.id))),
        }
        row.session.started |= op.status == AgentStatus::Working;
        self.save_and_publish(&mut state, &mut row)
    }

    /// Starts the agent of a live task whose agent is not running. The login shell runs outside the state lock, so a
    /// slow profile stalls no other op; the task is read again after it, in case another focus or a move landed
    /// meanwhile.
    pub(crate) fn task_focus(&self, op: TaskFocus) -> Result<Revised, Error> {
        let (directory, harness) = {
            let mut state = lock(&self.state);
            let row = state.store.task(&op.task)?;
            if !row.state.is_live() {
                return Ok(Revised { rev: row.rev });
            }
            self.watch_checkout(&mut state, &row)?;
            if state.agent_running(&row.id) {
                return Ok(Revised { rev: row.rev });
            }
            (PathBuf::from(&row.directory), state.store.harness(&row.harness)?)
        };
        let started = harness.map(|harness| login::resolve(&directory).map(|env| (harness, env))).transpose()?;
        let mut state = lock(&self.state);
        let mut row = state.store.task(&op.task)?;
        if !row.state.is_live() || state.agent_running(&row.id) {
            return Ok(Revised { rev: row.rev });
        }
        state.forget_agent(&row.id);
        let slot = match started {
            Some((harness, env)) => self.start_agent(&mut state, &mut row, harness, op.args, env)?,
            None => AgentSlot::HarnessUnknown,
        };
        let rev = state.store.save_task(&mut row)?;
        state.agents.insert(row.id.clone(), slot);
        self.publish_task(&state, &row);
        Ok(Revised { rev })
    }

    /// Looks the program up on the agent's own PATH, so a harness counts as installed exactly when its agent can run.
    fn start_agent(
        &self,
        state: &mut State,
        row: &mut TaskRow,
        harness: Harness,
        focus_args: Vec<String>,
        env: Vec<(String, String)>,
    ) -> Result<AgentSlot, Error> {
        let directory = PathBuf::from(&row.directory);
        let (program, args) = harness.argv.split_first().expect("harness.set rejects an empty argv");
        let Ok(program) = which::which_in(program, Some(login::search_path(&env)?), &directory) else {
            return Ok(AgentSlot::HarnessNotInstalled { install: harness.install });
        };
        let session_args = if row.session.started { &harness.resume } else { &harness.start };
        let argv = std::iter::once(program.to_string_lossy().into_owned())
            .chain(args.iter().cloned())
            .chain(harness::args(harness.hooks, &self.octopus))
            .chain(focus_args)
            .chain(session_args.iter().map(|arg| arg.replace(harness::SESSION, &row.session.id.0)))
            .collect();
        let panel = PanelRow { id: PtyId::random(), task: row.id.clone(), name: AGENT_PANEL.to_string() };
        let terminal = self.spawn_panel(state, &panel, argv, &directory, env)?;
        row.session.started |= harness.hooks == Hooks::None && harness::names_session(&harness.start);
        Ok(AgentSlot::Spawned { terminal, status: AgentStatus::Idle, since: now() })
    }

    /// Stops every agent that has been idle, silent and unshown for `AGENT_RECLAIM_MS` at `now` and runs no detached
    /// descendant, returning their tasks. A stopped agent ends like any other, keeping its panel until the next focus
    /// resumes its session. Activity is read outside the state lock, and the stop is sent under it after a re-check;
    /// an agent whose activity the host cannot tell keeps running.
    pub fn reclaim_agents(&self, now: i64) -> Vec<TaskId> {
        let candidates: Vec<(TaskId, Arc<Terminal>)> = {
            let state = lock(&self.state);
            state.agents.keys().filter_map(|task| Some((task.clone(), state.idle_unshown_agent(task, now)?))).collect()
        };
        let mut reclaimed = Vec::new();
        for (task, terminal) in candidates {
            let activity = match terminal.activity() {
                Ok(activity) => activity,
                Err(error) if terminal.running() && !matches!(error, Error::InvalidArgs { .. }) => {
                    report!("read the activity of the agent of {task}: {error}");
                    continue;
                }
                Err(_) => continue,
            };
            if activity.detached_descendant || now - activity.output_at < AGENT_RECLAIM_MS {
                continue;
            }
            let state = lock(&self.state);
            if state.idle_unshown_agent(&task, now).is_some_and(|current| current.id == terminal.id) {
                match terminal.terminate() {
                    Ok(()) => reclaimed.push(task),
                    Err(error) => report!("stop the idle agent of {task}: {error}"),
                }
            }
        }
        reclaimed
    }
}
