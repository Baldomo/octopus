use octopus_proto::types::TaskState;

use crate::store::TaskRow;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveEffect {
    Nothing,
    StopProcesses,
}

pub fn ended_at(state: TaskState, now: i64) -> Option<i64> {
    (!state.is_live()).then_some(now)
}

pub fn apply_move(task: &mut TaskRow, to: TaskState, now: i64) -> MoveEffect {
    let from = task.state;
    task.state = to;
    task.ended_at = ended_at(to, now);
    task.archived_at = None;
    if from.is_live() && !to.is_live() { MoveEffect::StopProcesses } else { MoveEffect::Nothing }
}

#[cfg(test)]
mod tests {
    use super::*;
    use octopus_proto::types::{Checkout, TaskId, WorkspaceId};

    fn task(state: TaskState) -> TaskRow {
        TaskRow {
            id: TaskId("tsk_1".into()),
            workspace: WorkspaceId("wsp_1".into()),
            parent: None,
            title: "t".into(),
            state,
            checkout: Checkout::Root,
            harness: "h".into(),
            session: None,
            directory: "/w".into(),
            branch: None,
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
