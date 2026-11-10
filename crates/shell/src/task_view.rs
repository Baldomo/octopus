use std::collections::BTreeMap;

use octopus_client::cache::Cache;
use octopus_proto::types::{LockHolder, Panel, Task};
use serde::ser::Error as _;
use serde::{Serialize, Serializer};
use ts_rs::TS;

use crate::task_lists::{Glyph, TaskRef, Waking, section_of};

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
pub struct TaskView {
    pub key: String,
    pub at: TaskRef,
    pub section: String,
    /// The workspace, then each ancestor root first, then the task.
    pub crumbs: Vec<String>,
    pub live: bool,
    pub glyph: Glyph,
    /// The task with its panels taken out: they reach the UI through `panels` alone.
    #[serde(serialize_with = "without_panels")]
    #[ts(type = "Omit<Task, \"panels\">")]
    pub task: Task,
    pub panels: Vec<PanelView>,
}

fn without_panels<S: Serializer>(task: &Task, serializer: S) -> Result<S::Ok, S::Error> {
    let mut task = serde_json::to_value(task).map_err(S::Error::custom)?;
    task.as_object_mut().expect("a task serializes to an object").remove("panels");
    task.serialize(serializer)
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
pub struct PanelView {
    #[serde(flatten)]
    #[ts(flatten)]
    pub panel: Panel,
    /// Whether this client holds the panel's lock.
    pub held_here: bool,
}

fn crumbs(cache: &Cache, task: &Task) -> Vec<String> {
    let mut ancestors = Vec::new();
    let mut parent = task.parent.as_ref().and_then(|parent| cache.tasks.get(parent));
    while let Some(ancestor) = parent {
        ancestors.push(ancestor.title.clone());
        parent = ancestor.parent.as_ref().and_then(|parent| cache.tasks.get(parent));
    }
    std::iter::once(section_of(cache, task).to_string())
        .chain(ancestors.into_iter().rev())
        .chain(std::iter::once(task.title.clone()))
        .collect()
}

/// Each task of every server, with `held_here` telling whether this client holds a lock on a server.
pub(crate) fn task_views(
    caches: &BTreeMap<String, Cache>,
    waking: &Waking,
    held_here: impl Fn(&str, &LockHolder) -> bool,
) -> BTreeMap<String, TaskView> {
    let held_here = &held_here;
    caches
        .iter()
        .flat_map(|(server, cache)| {
            cache.tasks.values().map(move |task| {
                let panels = task
                    .panels
                    .iter()
                    .map(|panel| PanelView {
                        held_here: panel.lock_holder.as_ref().is_some_and(|holder| held_here(server, holder)),
                        panel: panel.clone(),
                    })
                    .collect();
                let at = TaskRef { server: server.clone(), task: task.id.clone() };
                let view = TaskView {
                    key: at.key(),
                    section: section_of(cache, task).to_string(),
                    crumbs: crumbs(cache, task),
                    live: task.state.is_live(),
                    glyph: Glyph::shown(&at, task, waking),
                    task: task.clone(),
                    panels,
                    at,
                };
                (view.key.clone(), view)
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::task_lists::tests::{cache, task};
    use octopus_proto::types::TaskState;

    #[test]
    fn crumbs_run_from_the_workspace_through_each_ancestor_to_the_task() {
        let caches = BTreeMap::from([(
            "mac-mini".into(),
            cache(
                &[("wsp_a", "octopus")],
                vec![
                    task("tsk_1", "wsp_a", None, "protocol v1", TaskState::Pinned),
                    task("tsk_2", "wsp_a", Some("tsk_1"), "strict serde", TaskState::Completed),
                    task("tsk_3", "wsp_a", Some("tsk_2"), "envelope", TaskState::Pinned),
                ],
            ),
        )]);
        let views = task_views(&caches, &Waking::new(), |_, _| false);
        assert_eq!(views["mac-mini/tsk_3"].crumbs, ["octopus", "protocol v1", "strict serde", "envelope"]);
    }
}
