//! The workspaces a server serves: adding, renaming, coloring and removing them.

use std::path::{Path, PathBuf};

use octopus_proto::ops::{
    Revised, WorkspaceAdd, WorkspaceCreated, WorkspaceRemove, WorkspaceRename, WorkspaceSetColor,
};
use octopus_proto::types::Checkout;
use octopus_proto::{Error, Event};

use crate::{Server, checkouts, lock, nonempty};

/// A workspace root: an absolute path to a directory, as its canonical spelling.
fn directory(root: &str) -> Result<String, Error> {
    if !Path::new(root).is_absolute() {
        return Err(Error::invalid(format!("root {root} is not an absolute path")));
    }
    let canonical = std::fs::canonicalize(root).map_err(|error| Error::invalid(format!("root {root}: {error}")))?;
    if !canonical.is_dir() {
        return Err(Error::invalid(format!("root {root} is not a directory")));
    }
    Ok(canonical.to_string_lossy().into_owned())
}

impl Server {
    pub(crate) fn workspace_add(&self, op: WorkspaceAdd) -> Result<WorkspaceCreated, Error> {
        nonempty("a workspace name", &op.name)?;
        let root = directory(&op.root)?;
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
        let root = PathBuf::from(&workspace.root);
        for task in tasks.iter().filter(|task| matches!(task.checkout, Checkout::Worktree { .. })) {
            let directory = PathBuf::from(&task.directory);
            if directory.exists() {
                checkouts::remove(&root, &directory)?;
            }
        }
        let (rerooted, rev) = state.store.remove_workspace(&workspace.id)?;
        for row in &rerooted {
            self.publish_task(&state, row);
        }
        self.publish(&state, rev, Event::WorkspaceRemoved { workspace: workspace.id });
        Ok(Revised { rev })
    }
}
