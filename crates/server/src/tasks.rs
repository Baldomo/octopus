//! The task tree and its states: creating, moving, nesting, renaming and archiving tasks, and the view each publishes.

use std::path::{Path, PathBuf};

use octopus_proto::Error;
use octopus_proto::ops::{
    Revised, TaskArchive, TaskCreate, TaskCreated, TaskMove, TaskRename, TaskSetParent, TaskSetSession,
};
use octopus_proto::types::{
    Agent, Checkout, CheckoutKind, FRESH_SESSION, Session, SessionId, SessionStart, Task, TaskId, TaskState,
};

use crate::agents::AgentSlot;
use crate::store::TaskRow;
use crate::{Server, State, checkouts, lock, nonempty, now, report};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MoveEffect {
    Nothing,
    StopProcesses,
}

/// A session no agent has run in yet, named by a UUID v4.
fn fresh_session() -> Session {
    Session { id: SessionId(uuid::Uuid::new_v4().to_string()), started: false }
}

pub fn ended_at(state: TaskState, now: i64) -> Option<i64> {
    (!state.is_live()).then_some(now)
}

pub(crate) fn apply_move(task: &mut TaskRow, to: TaskState, now: i64) -> MoveEffect {
    let from = task.state;
    task.state = to;
    task.ended_at = ended_at(to, now);
    task.archived_at = None;
    if from.is_live() && !to.is_live() { MoveEffect::StopProcesses } else { MoveEffect::Nothing }
}

impl State {
    pub(crate) fn view(&self, row: &TaskRow) -> Task {
        let agent = match self.agents.get(&row.id) {
            None => Agent::NotStarted,
            Some(AgentSlot::Spawned { terminal, status, .. }) if terminal.running() => {
                Agent::Running { status: *status }
            }
            Some(AgentSlot::Spawned { .. }) => Agent::NotStarted,
            Some(AgentSlot::HarnessNotInstalled { install }) => Agent::HarnessNotInstalled { install: install.clone() },
            Some(AgentSlot::HarnessUnknown) => Agent::HarnessUnknown,
        };
        Task {
            id: row.id.clone(),
            workspace: row.workspace.clone(),
            parent: row.parent.clone(),
            title: row.title.clone(),
            state: row.state,
            checkout: row.checkout.clone(),
            harness: row.harness.clone(),
            session: row.session.clone(),
            directory: row.directory.clone(),
            ended_at: row.ended_at,
            archived_at: row.archived_at,
            rev: row.rev,
            agent,
            panels: self.panels(&row.id),
            browsers: self.browsers_of(&row.id).to_vec(),
        }
    }
}

impl Server {
    pub(crate) fn task_create(&self, op: TaskCreate) -> Result<TaskCreated, Error> {
        let session = match op.session {
            SessionStart::Fresh => fresh_session(),
            SessionStart::Resume(id) => {
                nonempty("a session id", &id.0)?;
                Session { id, started: true }
            }
        };
        let workspace = {
            let state = lock(&self.state);
            if let Some(parent) = &op.parent {
                state.store.task(parent)?;
            }
            state.store.workspace(&op.workspace)?
        };
        let id = TaskId::random();
        let root = PathBuf::from(&workspace.root);
        let branch = || checkouts::branch_for(&root, &op.title, &id);
        let checkout = match op.checkout {
            CheckoutKind::Root => Checkout::Root,
            CheckoutKind::Worktree => Checkout::Worktree { branch: branch()? },
            CheckoutKind::Copy => Checkout::Copy { branch: branch()? },
        };
        let directory = match checkout {
            Checkout::Root => root,
            Checkout::Worktree { .. } | Checkout::Copy { .. } => {
                let directory = self.worktrees.join(&workspace.name).join(&id.0);
                checkouts::create(&root, &directory, &checkout)?;
                directory
            }
        };
        let mut state = lock(&self.state);
        let row = state.store.create_task(TaskRow {
            id,
            workspace: op.workspace,
            parent: op.parent,
            title: op.title,
            state: op.state,
            checkout,
            harness: op.harness,
            session,
            directory: directory.to_string_lossy().to_string(),
            ended_at: ended_at(op.state, now()),
            archived_at: None,
            rev: 0,
        })?;
        self.publish_task(&state, &row);
        Ok(TaskCreated { task: row.id, rev: row.rev })
    }

    /// A reopened task whose worktree is gone gets it back, made outside the lock since a copy clones its files.
    pub(crate) fn task_move(&self, op: TaskMove) -> Result<Revised, Error> {
        let missing = {
            let state = lock(&self.state);
            let row = state.store.task(&op.task)?;
            let directory = PathBuf::from(&row.directory);
            if !row.state.is_live() && op.state.is_live() && !directory.exists() {
                Some((PathBuf::from(state.store.workspace(&row.workspace)?.root), directory, row.checkout))
            } else {
                None
            }
        };
        if let Some((root, directory, checkout)) = missing {
            checkouts::create(&root, &directory, &checkout)?;
        }
        let mut state = lock(&self.state);
        let mut row = state.store.task(&op.task)?;
        let reopening = !row.state.is_live() && op.state.is_live();
        if reopening && !Path::new(&row.directory).exists() {
            return Err(Error::conflict(format!("{}'s worktree went while it was made; move it again", row.id)));
        }
        let effect = apply_move(&mut row, op.state, now());
        let rev = state.store.save_task(&mut row)?;
        if reopening {
            state.forget_agent(&row.id);
        }
        if effect == MoveEffect::StopProcesses {
            state.watchers.remove(&row.id);
            for (name, terminal) in state.terminals(&row.id) {
                if let Err(error) = terminal.terminate() {
                    report!("stop the {name} panel of {}: {error}", row.id);
                }
            }
        }
        self.publish_task(&state, &row);
        Ok(Revised { rev })
    }

    pub(crate) fn task_set_parent(&self, op: TaskSetParent) -> Result<Revised, Error> {
        let mut state = lock(&self.state);
        let mut row = state.store.task(&op.task)?;
        if let Some(parent) = &op.parent {
            let mut above = state.store.task(parent)?;
            if above.workspace != row.workspace {
                return Err(Error::conflict(format!("{parent} is in another workspace than {}", row.id)));
            }
            loop {
                if above.id == row.id {
                    return Err(Error::conflict(format!("{} cannot nest under itself or its own subtask", row.id)));
                }
                let Some(next) = &above.parent else { break };
                above = state.store.task(next)?;
            }
        }
        row.parent = op.parent;
        let rev = state.store.save_task(&mut row)?;
        self.publish_task(&state, &row);
        Ok(Revised { rev })
    }

    pub(crate) fn task_rename(&self, op: TaskRename) -> Result<Revised, Error> {
        nonempty("a task title", &op.title)?;
        let mut state = lock(&self.state);
        let mut row = state.store.task(&op.task)?;
        row.title = op.title;
        let rev = state.store.save_task(&mut row)?;
        self.publish_task(&state, &row);
        Ok(Revised { rev })
    }

    pub(crate) fn task_set_session(&self, op: TaskSetSession) -> Result<Revised, Error> {
        nonempty("a session id", &op.session.0)?;
        if op.session.0 == FRESH_SESSION {
            return Err(Error::invalid(format!("{FRESH_SESSION} names no existing session")));
        }
        let mut state = lock(&self.state);
        let mut row = state.store.task(&op.task)?;
        row.session = Session { id: op.session, started: true };
        let rev = state.store.save_task(&mut row)?;
        state.forget_agent(&row.id);
        self.publish_task(&state, &row);
        Ok(Revised { rev })
    }

    pub(crate) fn task_archive(&self, op: TaskArchive) -> Result<Revised, Error> {
        let mut state = lock(&self.state);
        let mut row = state.store.task(&op.task)?;
        if row.state.is_live() {
            return Err(Error::conflict(format!("{} is live; end it before archiving it", row.id)));
        }
        row.archived_at = Some(now());
        let rev = state.store.save_task(&mut row)?;
        self.publish_task(&state, &row);
        Ok(Revised { rev })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use octopus_proto::types::WorkspaceId;

    fn task(state: TaskState) -> TaskRow {
        TaskRow {
            id: TaskId("tsk_1".into()),
            workspace: WorkspaceId("wsp_1".into()),
            parent: None,
            title: "t".into(),
            state,
            checkout: Checkout::Root,
            harness: "h".into(),
            session: fresh_session(),
            directory: "/w".into(),
            ended_at: ended_at(state, 1),
            archived_at: None,
            rev: 0,
        }
    }

    #[test]
    fn every_move_follows_the_transition_table() {
        let stop = MoveEffect::StopProcesses;
        let none = MoveEffect::Nothing;
        let table = [
            ("live -> live", TaskState::Pinned, TaskState::Temporary, None, none),
            ("live -> ended", TaskState::Pinned, TaskState::Completed, Some(9), stop),
            ("ended -> ended", TaskState::Completed, TaskState::Cancelled, Some(9), none),
            ("ended -> live", TaskState::Cancelled, TaskState::Pinned, None, none),
        ];
        for (name, from, to, ended_at, effect) in table {
            let mut row = task(from);
            assert_eq!(apply_move(&mut row, to, 9), effect, "{name}");
            assert_eq!((row.state, row.ended_at), (to, ended_at), "{name}");
        }
    }
}
