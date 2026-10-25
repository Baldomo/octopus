use std::collections::{BTreeMap, HashMap};

use octopus_client::cache::Cache;
use octopus_proto::types::{Agent, AgentStatus, Task, TaskId, TaskState, WorkspaceId};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::saturate;

/// A task anywhere in the client: task ids are unique only within their server.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct TaskRef {
    pub server: String,
    pub task: TaskId,
}

impl TaskRef {
    pub fn key(&self) -> String {
        format!("{}/{}", self.server, self.task)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum Glyph {
    Working,
    Waiting,
    Idle,
    Completed,
    Cancelled,
}

impl Glyph {
    pub fn of(task: &Task) -> Self {
        match (task.state, &task.agent) {
            (TaskState::Completed, _) => Self::Completed,
            (TaskState::Cancelled, _) => Self::Cancelled,
            (_, Agent::Running { status: AgentStatus::Working }) => Self::Working,
            (_, Agent::Running { status: AgentStatus::Waiting }) => Self::Waiting,
            _ => Self::Idle,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
pub struct Row {
    pub key: String,
    pub at: TaskRef,
    pub title: String,
    pub depth: u32,
    /// The parent's title, for a subtask that sits apart from its parent.
    pub via: Option<String>,
    pub glyph: Glyph,
    pub subtasks: u32,
}

/// One state's tasks in one section: a sidebar group or a board column.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
pub struct Column {
    pub state: TaskState,
    /// Whether the column's state is live, which puts it in the sidebar too.
    pub live: bool,
    pub rows: Vec<Row>,
}

/// Every server's workspaces that share one name.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
pub struct Section {
    pub name: String,
    pub live: u32,
    /// One per state, in `TaskState::ALL` order, empty ones included.
    pub columns: Vec<Column>,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
pub struct WorkspaceRef {
    pub server: String,
    pub workspace: WorkspaceId,
    pub name: String,
    /// The directory on the server's machine.
    pub root: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
pub struct Lists {
    pub sections: Vec<Section>,
    pub workspaces: Vec<WorkspaceRef>,
}

pub fn section_of<'a>(cache: &'a Cache, task: &'a Task) -> &'a str {
    cache.workspaces.get(&task.workspace).map_or(task.workspace.0.as_str(), |workspace| workspace.name.as_str())
}

struct Placed<'a> {
    at: TaskRef,
    task: &'a Task,
    section: &'a str,
}

fn column_index(state: TaskState) -> usize {
    TaskState::ALL.iter().position(|candidate| *candidate == state).expect("ALL lists every state")
}

pub fn lists(caches: &BTreeMap<String, Cache>) -> Lists {
    let mut placed: Vec<Placed> = caches
        .iter()
        .flat_map(|(server, cache)| {
            cache.tasks.values().map(move |task| Placed {
                at: TaskRef { server: server.clone(), task: task.id.clone() },
                task,
                section: section_of(cache, task),
            })
        })
        .collect();
    placed.sort_by(|a, b| (a.task.title.to_lowercase(), &a.at).cmp(&(b.task.title.to_lowercase(), &b.at)));

    let index: HashMap<(&str, &TaskId), usize> =
        placed.iter().enumerate().map(|(i, task)| ((task.at.server.as_str(), &task.at.task), i)).collect();
    let parents: Vec<Option<usize>> = placed
        .iter()
        .map(|task| task.task.parent.as_ref().and_then(|parent| index.get(&(task.at.server.as_str(), parent)).copied()))
        .collect();
    let archived = |i: usize| placed[i].task.archived_at.is_some();
    let nested_under = |i: usize| {
        parents[i].filter(|&parent| {
            !archived(parent)
                && placed[parent].section == placed[i].section
                && placed[parent].task.state == placed[i].task.state
        })
    };
    let mut subtasks = vec![0; placed.len()];
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); placed.len()];
    let mut roots: BTreeMap<&str, [Vec<usize>; TaskState::ALL.len()]> = caches
        .values()
        .flat_map(|cache| cache.workspaces.values().map(|workspace| (workspace.name.as_str(), Default::default())))
        .collect();
    for i in (0..placed.len()).filter(|&i| !archived(i)) {
        if let Some(parent) = parents[i] {
            subtasks[parent] += 1;
        }
        match nested_under(i) {
            Some(parent) => children[parent].push(i),
            None => roots.entry(placed[i].section).or_default()[column_index(placed[i].task.state)].push(i),
        }
    }

    let row = |i: usize, depth: u32| {
        let task = &placed[i];
        let apart = nested_under(i).is_none();
        Row {
            key: task.at.key(),
            at: task.at.clone(),
            title: task.task.title.clone(),
            depth,
            via: parents[i].filter(|_| apart).map(|parent| placed[parent].task.title.clone()),
            glyph: Glyph::of(task.task),
            subtasks: subtasks[i],
        }
    };
    let sections = roots
        .into_iter()
        .map(|(name, columns)| {
            let columns: Vec<Column> = TaskState::ALL
                .into_iter()
                .zip(columns)
                .map(|(state, roots)| {
                    let mut rows = Vec::new();
                    let mut stack: Vec<(usize, u32)> = roots.into_iter().rev().map(|i| (i, 0)).collect();
                    while let Some((i, depth)) = stack.pop() {
                        rows.push(row(i, depth));
                        stack.extend(children[i].iter().rev().map(|&child| (child, depth + 1)));
                    }
                    Column { state, live: state.is_live(), rows }
                })
                .collect();
            let live: usize = columns.iter().filter(|column| column.live).map(|column| column.rows.len()).sum();
            Section { name: name.to_string(), live: saturate(live), columns }
        })
        .collect();

    let workspaces = caches
        .iter()
        .flat_map(|(server, cache)| {
            cache.workspaces.values().map(move |workspace| WorkspaceRef {
                server: server.clone(),
                workspace: workspace.id.clone(),
                name: workspace.name.clone(),
                root: workspace.root.clone(),
            })
        })
        .collect();
    Lists { sections, workspaces }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use octopus_proto::types::{Checkout, Workspace};

    pub fn task(id: &str, workspace: &str, parent: Option<&str>, title: &str, state: TaskState) -> Task {
        Task {
            id: TaskId(id.into()),
            workspace: WorkspaceId(workspace.into()),
            parent: parent.map(|parent| TaskId(parent.into())),
            title: title.into(),
            state,
            checkout: Checkout::Root,
            harness: "claude-code".into(),
            session: None,
            directory: "/repo".into(),
            branch: None,
            focused_at: None,
            ended_at: None,
            archived_at: None,
            rev: 1,
            agent: Agent::NotStarted,
            panels: Vec::new(),
            browsers: Vec::new(),
        }
    }

    pub fn cache(workspaces: &[(&str, &str)], tasks: Vec<Task>) -> Cache {
        Cache {
            rev: 1,
            workspaces: workspaces
                .iter()
                .map(|(id, name)| {
                    let id = WorkspaceId((*id).into());
                    (id.clone(), Workspace { id, name: (*name).into(), root: "/repo".into() })
                })
                .collect(),
            tasks: tasks.into_iter().map(|task| (task.id.clone(), task)).collect(),
        }
    }

    fn rows(lists: &Lists, section: &str, state: TaskState) -> Vec<(String, u32, Option<String>)> {
        let section = lists.sections.iter().find(|candidate| candidate.name == section).expect("section");
        let column = section.columns.iter().find(|column| column.state == state).expect("column");
        column.rows.iter().map(|row| (row.title.clone(), row.depth, row.via.clone())).collect()
    }

    #[test]
    fn same_named_workspaces_on_two_servers_form_one_section() {
        let caches = BTreeMap::from([
            (
                "laptop".into(),
                cache(&[("wsp_a", "octopus")], vec![task("tsk_1", "wsp_a", None, "b", TaskState::Pinned)]),
            ),
            (
                "mac-mini".into(),
                cache(
                    &[("wsp_b", "octopus"), ("wsp_c", "dotfiles"), ("wsp_d", "scratch")],
                    vec![
                        task("tsk_1", "wsp_b", None, "a", TaskState::Pinned),
                        task("tsk_2", "wsp_c", None, "c", TaskState::Completed),
                    ],
                ),
            ),
        ]);
        let lists = lists(&caches);
        let names: Vec<(&str, u32)> =
            lists.sections.iter().map(|section| (section.name.as_str(), section.live)).collect();
        assert_eq!(names, [("dotfiles", 0), ("octopus", 2), ("scratch", 0)]);
        let servers: Vec<&str> = lists.sections[1].columns[0].rows.iter().map(|row| row.at.server.as_str()).collect();
        assert_eq!(servers, ["mac-mini", "laptop"]);
        assert_eq!(lists.workspaces.len(), 4);
    }

    #[test]
    fn a_subtask_nests_only_under_a_parent_in_its_own_group() {
        let caches = BTreeMap::from([(
            "mac-mini".into(),
            cache(
                &[("wsp_a", "octopus"), ("wsp_b", "acme")],
                vec![
                    task("tsk_1", "wsp_a", None, "protocol v1", TaskState::Pinned),
                    task("tsk_2", "wsp_a", Some("tsk_1"), "strict serde", TaskState::Pinned),
                    task("tsk_3", "wsp_a", Some("tsk_2"), "deny unknown", TaskState::Pinned),
                    task("tsk_4", "wsp_a", Some("tsk_1"), "pairing", TaskState::Temporary),
                    task("tsk_5", "wsp_a", None, "a sibling", TaskState::Pinned),
                    task("tsk_6", "wsp_b", Some("tsk_1"), "elsewhere", TaskState::Pinned),
                ],
            ),
        )]);
        let lists = lists(&caches);
        assert_eq!(
            rows(&lists, "octopus", TaskState::Pinned),
            [
                ("a sibling".into(), 0, None),
                ("protocol v1".into(), 0, None),
                ("strict serde".into(), 1, None),
                ("deny unknown".into(), 2, None),
            ]
        );
        assert_eq!(rows(&lists, "octopus", TaskState::Temporary), [("pairing".into(), 0, Some("protocol v1".into()))]);
        assert_eq!(rows(&lists, "acme", TaskState::Pinned), [("elsewhere".into(), 0, Some("protocol v1".into()))]);
        let protocol = &lists.sections[1].columns[0].rows[1];
        assert_eq!((protocol.title.as_str(), protocol.subtasks), ("protocol v1", 3));
    }

    #[test]
    fn an_archived_task_sits_nowhere_and_its_subtask_sits_apart() {
        let mut parent = task("tsk_1", "wsp_a", None, "protocol v1", TaskState::Completed);
        parent.archived_at = Some(1);
        let child = task("tsk_2", "wsp_a", Some("tsk_1"), "strict serde", TaskState::Completed);
        let caches = BTreeMap::from([("mac-mini".into(), cache(&[("wsp_a", "octopus")], vec![parent, child]))]);
        let lists = lists(&caches);
        assert_eq!(
            rows(&lists, "octopus", TaskState::Completed),
            [("strict serde".into(), 0, Some("protocol v1".into()))]
        );
    }

    #[test]
    fn a_task_shows_its_agent_while_live_and_how_it_ended_after() {
        let mut working = task("tsk_1", "wsp_a", None, "t", TaskState::Temporary);
        working.agent = Agent::Running { status: AgentStatus::Working };
        let mut cancelled = working.clone();
        cancelled.state = TaskState::Cancelled;
        let mut missing = task("tsk_1", "wsp_a", None, "t", TaskState::Pinned);
        missing.agent = Agent::HarnessUnknown;
        assert_eq!(
            [Glyph::of(&working), Glyph::of(&cancelled), Glyph::of(&missing)],
            [Glyph::Working, Glyph::Cancelled, Glyph::Idle]
        );
    }
}
