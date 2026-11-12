//! The workspaces a server files tasks in: adding, renaming, coloring and removing them.

use std::path::Path;

use octopus_proto::ops::{
    Revised, WorkspaceAdd, WorkspaceCreated, WorkspaceRemove, WorkspaceRename, WorkspaceSetColor,
};
use octopus_proto::{Error, Event};

use crate::{Server, folders, lock, nonempty, worktrees};

impl Server {
    pub(crate) fn workspace_add(&self, op: WorkspaceAdd) -> Result<WorkspaceCreated, Error> {
        nonempty("a workspace name", &op.name)?;
        let root = folders::directory("root", &op.root)?;
        let mut state = lock(&self.state);
        let (workspace, rev) = state.store.add_workspace(&op.name, &root, op.color)?;
        let id = workspace.id.clone();
        self.publish(&state, rev, Event::WorkspaceUpdated { workspace });
        Ok(WorkspaceCreated { workspace: id, rev })
    }

    pub(crate) fn workspace_rename(&self, op: WorkspaceRename) -> Result<Revised, Error> {
        nonempty("a workspace name", &op.name)?;
        let mut state = lock(&self.state);
        let (workspace, rev) = state.store.rename_workspace(&op.workspace, &op.name)?;
        self.publish(&state, rev, Event::WorkspaceUpdated { workspace });
        Ok(Revised { rev })
    }

    pub(crate) fn workspace_set_color(&self, op: WorkspaceSetColor) -> Result<Revised, Error> {
        let mut state = lock(&self.state);
        let (workspace, rev) = state.store.color_workspace(&op.workspace, op.color)?;
        self.publish(&state, rev, Event::WorkspaceUpdated { workspace });
        Ok(Revised { rev })
    }

    /// Runs under the state lock throughout, so no task starts in the workspace while it goes.
    pub(crate) fn workspace_remove(&self, op: WorkspaceRemove) -> Result<Revised, Error> {
        let mut state = lock(&self.state);
        let workspace = state.store.workspace(&op.workspace)?;
        let tasks = state.store.workspace_tasks(&workspace.id)?;
        let live = tasks.iter().filter(|task| task.state.is_live()).count();
        if live > 0 {
            return Err(Error::conflict(format!("{} has {live} live tasks; end them first", workspace.name)));
        }
        for task in &tasks {
            state.watchers.remove(&task.id);
            state.browsers.remove(&task.id);
            state.forget_terminals(&task.id);
        }
        for worktree in tasks.iter().filter_map(|task| task.worktree.as_ref()) {
            if Path::new(&worktree.path).exists() {
                worktrees::remove(&worktree.path)?;
            }
        }
        let rev = state.store.remove_workspace(&workspace.id)?;
        self.publish(&state, rev, Event::WorkspaceRemoved { workspace: workspace.id });
        Ok(Revised { rev })
    }
}
