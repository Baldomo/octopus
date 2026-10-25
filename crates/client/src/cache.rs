use std::collections::BTreeMap;

use octopus_proto::ops::Snapshot;
use octopus_proto::types::{Task, TaskId, Workspace, WorkspaceId};
use octopus_proto::{Event, Push};

#[derive(Debug, Default, Clone, PartialEq)]
pub struct Cache {
    pub rev: u64,
    pub workspaces: BTreeMap<WorkspaceId, Workspace>,
    pub tasks: BTreeMap<TaskId, Task>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PushEffect {
    Applied,
    Stale,
    NotState,
}

impl Cache {
    pub fn from_snapshot(snapshot: Snapshot) -> Self {
        Self {
            rev: snapshot.rev,
            workspaces: snapshot.workspaces.into_iter().map(|workspace| (workspace.id.clone(), workspace)).collect(),
            tasks: snapshot.tasks.into_iter().map(|task| (task.id.clone(), task)).collect(),
        }
    }

    pub fn apply(&mut self, push: &Push) -> PushEffect {
        match &push.event {
            Event::FileChanged { .. } => return PushEffect::NotState,
            _ if push.rev <= self.rev => return PushEffect::Stale,
            Event::TaskUpdated { task } => {
                self.tasks.insert(task.id.clone(), task.as_ref().clone());
            }
            Event::WorkspaceUpdated { workspace } => {
                self.workspaces.insert(workspace.id.clone(), workspace.clone());
            }
            Event::WorkspaceRemoved { workspace } => {
                self.workspaces.remove(workspace);
                self.tasks.retain(|_, task| task.workspace != *workspace);
            }
            Event::HarnessUpdated { .. } => {}
        }
        self.rev = push.rev;
        PushEffect::Applied
    }
}
