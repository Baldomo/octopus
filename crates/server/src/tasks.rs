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
    if !from.is_live() && to.is_live() {
        task.focused_at = None;
    }
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
            directory: "/w".into(),
            branch: None,
            focused_at: Some(1),
            ended_at: ended_at(state, 1),
            rev: 0,
        }
    }

    #[test]
    fn every_move_follows_the_transition_table() {
        let stop = MoveEffect::StopProcesses;
        let none = MoveEffect::Nothing;
        let table = [
            ("live -> live", TaskState::Pinned, TaskState::Temporary, Some(1), None, none),
            ("live -> ended", TaskState::Pinned, TaskState::Completed, Some(1), Some(9), stop),
            ("ended -> ended", TaskState::Completed, TaskState::Cancelled, Some(1), Some(9), none),
            ("ended -> live", TaskState::Cancelled, TaskState::Pinned, None, None, none),
        ];
        for (name, from, to, focused_at, ended_at, effect) in table {
            let mut row = task(from);
            assert_eq!(apply_move(&mut row, to, 9), effect, "{name}");
            assert_eq!((row.state, row.focused_at, row.ended_at), (to, focused_at, ended_at), "{name}");
        }
    }
}
