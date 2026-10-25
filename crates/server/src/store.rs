use std::collections::{HashMap, HashSet};
use std::path::Path;

use octopus_proto::Error;
use octopus_proto::device::{DeviceKey, PublicKey};
use octopus_proto::types::{
    BrowserId, BrowserPanel, Checkout, PtyId, TaskId, TaskState, Workspace, WorkspaceId, from_wire, to_wire,
};
use rusqlite::types::Type;
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};
use serde::de::DeserializeOwned;

const SCHEMA: &str = "
PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value INTEGER NOT NULL);
INSERT OR IGNORE INTO meta (key, value) VALUES ('rev', 0);
CREATE TABLE IF NOT EXISTS workspaces (id TEXT PRIMARY KEY, name TEXT NOT NULL UNIQUE, root TEXT NOT NULL UNIQUE);
CREATE TABLE IF NOT EXISTS harnesses (id TEXT PRIMARY KEY, argv TEXT NOT NULL, install TEXT NOT NULL, hooks INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS tasks (
    id TEXT PRIMARY KEY,
    workspace TEXT NOT NULL REFERENCES workspaces(id),
    parent TEXT REFERENCES tasks(id),
    title TEXT NOT NULL,
    state TEXT NOT NULL,
    checkout TEXT NOT NULL,
    harness TEXT NOT NULL,
    directory TEXT NOT NULL,
    branch TEXT,
    focused_at INTEGER,
    ended_at INTEGER,
    rev INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS terminals (
    task TEXT PRIMARY KEY REFERENCES tasks(id),
    id TEXT NOT NULL UNIQUE
);
CREATE TABLE IF NOT EXISTS browsers (
    task TEXT NOT NULL REFERENCES tasks(id),
    panel TEXT NOT NULL,
    url TEXT NOT NULL,
    PRIMARY KEY (task, panel)
);
CREATE TABLE IF NOT EXISTS device (id INTEGER PRIMARY KEY CHECK (id = 0), pkcs8 BLOB NOT NULL);
CREATE TABLE IF NOT EXISTS clients (name TEXT PRIMARY KEY, key TEXT NOT NULL UNIQUE);
";

/// A task's agent terminal, the one panel a task has.
#[derive(Debug, Clone, PartialEq)]
pub struct TerminalRow {
    pub id: PtyId,
    pub task: TaskId,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TaskRow {
    pub id: TaskId,
    pub workspace: WorkspaceId,
    pub parent: Option<TaskId>,
    pub title: String,
    pub state: TaskState,
    pub checkout: Checkout,
    pub harness: String,
    pub directory: String,
    pub branch: Option<String>,
    pub focused_at: Option<i64>,
    pub ended_at: Option<i64>,
    pub rev: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Harness {
    pub argv: Vec<String>,
    pub install: String,
    pub hooks: bool,
}

#[cfg(unix)]
fn owner_only(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    std::fs::OpenOptions::new().create(true).append(true).mode(0o600).open(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

/// A Windows profile's files are its user's own.
#[cfg(windows)]
fn owner_only(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

pub struct Store {
    db: Connection,
}

fn bump_rev(tx: &Transaction) -> Result<u64, Error> {
    tx.query_row("UPDATE meta SET value = value + 1 WHERE key = 'rev' RETURNING value", [], |row| row.get::<_, i64>(0))
        .map(i64::cast_unsigned)
        .map_err(Error::internal)
}

fn wire_column<T: DeserializeOwned>(row: &Row, column: &str) -> rusqlite::Result<T> {
    let text: String = row.get(column)?;
    from_wire(&text).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(0, Type::Text, format!("{column} = {text:?}: {error}").into())
    })
}

fn task_row(row: &Row) -> rusqlite::Result<TaskRow> {
    Ok(TaskRow {
        id: TaskId(row.get("id")?),
        workspace: WorkspaceId(row.get("workspace")?),
        parent: row.get::<_, Option<String>>("parent")?.map(TaskId),
        title: row.get("title")?,
        state: wire_column(row, "state")?,
        checkout: wire_column(row, "checkout")?,
        harness: row.get("harness")?,
        directory: row.get("directory")?,
        branch: row.get("branch")?,
        focused_at: row.get("focused_at")?,
        ended_at: row.get("ended_at")?,
        rev: row.get::<_, i64>("rev")?.cast_unsigned(),
    })
}

fn workspace_row(row: &Row) -> rusqlite::Result<Workspace> {
    Ok(Workspace { id: WorkspaceId(row.get("id")?), name: row.get("name")?, root: row.get("root")? })
}

impl Store {
    /// The store holds the server's private key, so only its owner may read it.
    pub fn open(path: &Path) -> Result<Self, Error> {
        owner_only(path).map_err(|error| Error::internal(format!("{}: {error}", path.display())))?;
        let db = Connection::open(path).map_err(Error::internal)?;
        db.execute_batch(SCHEMA).map_err(Error::internal)?;
        Ok(Self { db })
    }

    /// The server's own key, made on the store's first open.
    pub fn device_key(&mut self) -> Result<DeviceKey, Error> {
        let tx = self.db.transaction().map_err(Error::internal)?;
        let stored: Option<Vec<u8>> =
            tx.query_row("SELECT pkcs8 FROM device", [], |row| row.get(0)).optional().map_err(Error::internal)?;
        let key = match stored {
            Some(pkcs8) => DeviceKey::from_pkcs8(&pkcs8).map_err(Error::internal)?,
            None => {
                let key = DeviceKey::generate();
                tx.execute("INSERT INTO device (id, pkcs8) VALUES (0, ?1)", [key.pkcs8()]).map_err(Error::internal)?;
                key
            }
        };
        tx.commit().map_err(Error::internal)?;
        Ok(key)
    }

    /// A name pinned to another device stays its, until `pairing.forget` lets it go.
    pub fn refuse_namesake(&self, name: &str, key: &PublicKey) -> Result<(), Error> {
        let holder: Option<String> = self
            .db
            .query_row("SELECT key FROM clients WHERE name = ?1", [name], |row| row.get(0))
            .optional()
            .map_err(Error::internal)?;
        match holder {
            Some(holder) if holder != key.to_string() => Err(Error::conflict(format!(
                "the client name {name} is paired to another device's key; `pairing forget` it first"
            ))),
            _ => Ok(()),
        }
    }

    /// Replaces the key's earlier pin, so a re-paired client keeps one.
    pub fn pin_client(&mut self, name: &str, key: &PublicKey) -> Result<(), Error> {
        let key = key.to_string();
        let tx = self.db.transaction().map_err(Error::internal)?;
        tx.execute("DELETE FROM clients WHERE key = ?1", [&key]).map_err(Error::internal)?;
        tx.execute("INSERT INTO clients (name, key) VALUES (?1, ?2)", params![name, key]).map_err(Error::internal)?;
        tx.commit().map_err(Error::internal)
    }

    pub fn forget_client(&mut self, name: &str) -> Result<(), Error> {
        match self.db.execute("DELETE FROM clients WHERE name = ?1", [name]).map_err(Error::internal)? {
            0 => Err(Error::not_found(format!("a paired client named {name}"))),
            _ => Ok(()),
        }
    }

    pub fn client_pinned_to(&self, key: &PublicKey) -> Result<Option<String>, Error> {
        self.db
            .query_row("SELECT name FROM clients WHERE key = ?1", [key.to_string()], |row| row.get(0))
            .optional()
            .map_err(Error::internal)
    }

    pub fn rev(&self) -> Result<u64, Error> {
        self.db
            .query_row("SELECT value FROM meta WHERE key = 'rev'", [], |row| row.get::<_, i64>(0))
            .map(i64::cast_unsigned)
            .map_err(Error::internal)
    }

    pub fn add_workspace(&mut self, name: &str, root: &str) -> Result<(Workspace, u64), Error> {
        let tx = self.db.transaction().map_err(Error::internal)?;
        let taken: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM workspaces WHERE name = ?1 OR root = ?2)",
                params![name, root],
                |row| row.get(0),
            )
            .map_err(Error::internal)?;
        if taken {
            return Err(Error::conflict(format!("a workspace named {name} or rooted at {root} exists")));
        }
        let workspace = Workspace { id: WorkspaceId::random(), name: name.to_string(), root: root.to_string() };
        tx.execute(
            "INSERT INTO workspaces (id, name, root) VALUES (?1, ?2, ?3)",
            params![workspace.id.0, workspace.name, workspace.root],
        )
        .map_err(Error::internal)?;
        let rev = bump_rev(&tx)?;
        tx.commit().map_err(Error::internal)?;
        Ok((workspace, rev))
    }

    pub fn workspaces(&self) -> Result<Vec<Workspace>, Error> {
        let mut statement = self.db.prepare("SELECT * FROM workspaces ORDER BY name").map_err(Error::internal)?;
        let rows = statement.query_map([], workspace_row).map_err(Error::internal)?;
        rows.collect::<Result<_, _>>().map_err(Error::internal)
    }

    pub fn workspace(&self, id: &WorkspaceId) -> Result<Workspace, Error> {
        self.db
            .query_row("SELECT * FROM workspaces WHERE id = ?1", [&id.0], workspace_row)
            .optional()
            .map_err(Error::internal)?
            .ok_or_else(|| Error::not_found(id))
    }

    pub fn set_harness(&mut self, id: &str, harness: &Harness) -> Result<u64, Error> {
        let argv = serde_json::to_string(&harness.argv).map_err(Error::internal)?;
        let tx = self.db.transaction().map_err(Error::internal)?;
        tx.execute(
            "INSERT INTO harnesses (id, argv, install, hooks) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET argv = excluded.argv, install = excluded.install, hooks = excluded.hooks",
            params![id, argv, harness.install, harness.hooks],
        )
        .map_err(Error::internal)?;
        let rev = bump_rev(&tx)?;
        tx.commit().map_err(Error::internal)?;
        Ok(rev)
    }

    pub fn harness(&self, id: &str) -> Result<Option<Harness>, Error> {
        let row: Option<(String, String, bool)> = self
            .db
            .query_row("SELECT argv, install, hooks FROM harnesses WHERE id = ?1", [id], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .optional()
            .map_err(Error::internal)?;
        row.map(|(argv, install, hooks)| {
            Ok(Harness { argv: serde_json::from_str(&argv).map_err(Error::internal)?, install, hooks })
        })
        .transpose()
    }

    pub fn create_task(&mut self, mut row: TaskRow) -> Result<TaskRow, Error> {
        let tx = self.db.transaction().map_err(Error::internal)?;
        row.rev = bump_rev(&tx)?;
        tx.execute(
            "INSERT INTO tasks (id, workspace, parent, title, state, checkout, harness, directory, branch, focused_at, ended_at, rev)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                row.id.0,
                row.workspace.0,
                row.parent.as_ref().map(|parent| &parent.0),
                row.title,
                to_wire(&row.state),
                to_wire(&row.checkout),
                row.harness,
                row.directory,
                row.branch,
                row.focused_at,
                row.ended_at,
                row.rev.cast_signed()
            ],
        )
        .map_err(Error::internal)?;
        tx.commit().map_err(Error::internal)?;
        Ok(row)
    }

    pub fn task(&self, id: &TaskId) -> Result<TaskRow, Error> {
        self.db
            .query_row("SELECT * FROM tasks WHERE id = ?1", [&id.0], task_row)
            .optional()
            .map_err(Error::internal)?
            .ok_or_else(|| Error::not_found(id))
    }

    pub fn tasks(&self) -> Result<Vec<TaskRow>, Error> {
        let mut statement = self.db.prepare("SELECT * FROM tasks ORDER BY rowid").map_err(Error::internal)?;
        let rows = statement.query_map([], task_row).map_err(Error::internal)?;
        rows.collect::<Result<_, _>>().map_err(Error::internal)
    }

    pub fn worktrees_ended_by(&self, cutoff: i64) -> Result<Vec<TaskRow>, Error> {
        let mut statement = self
            .db
            .prepare("SELECT * FROM tasks WHERE checkout = 'worktree' AND ended_at IS NOT NULL AND ended_at <= ?1")
            .map_err(Error::internal)?;
        let rows = statement.query_map([cutoff], task_row).map_err(Error::internal)?;
        rows.collect::<Result<_, _>>().map_err(Error::internal)
    }

    pub fn save_task(&mut self, row: &mut TaskRow) -> Result<u64, Error> {
        let tx = self.db.transaction().map_err(Error::internal)?;
        row.rev = bump_rev(&tx)?;
        tx.execute(
            "UPDATE tasks SET state = ?2, focused_at = ?3, ended_at = ?4, rev = ?5 WHERE id = ?1",
            params![row.id.0, to_wire(&row.state), row.focused_at, row.ended_at, row.rev.cast_signed()],
        )
        .map_err(Error::internal)?;
        tx.commit().map_err(Error::internal)?;
        Ok(row.rev)
    }

    /// Moves a task to a new rev without changing its fields, for changes the task view derives.
    pub fn touch_task(&mut self, id: &TaskId) -> Result<TaskRow, Error> {
        let tx = self.db.transaction().map_err(Error::internal)?;
        let rev = bump_rev(&tx)?;
        let row = tx
            .query_row(
                "UPDATE tasks SET rev = ?2 WHERE id = ?1 RETURNING *",
                params![id.0, rev.cast_signed()],
                task_row,
            )
            .optional()
            .map_err(Error::internal)?
            .ok_or_else(|| Error::not_found(id))?;
        tx.commit().map_err(Error::internal)?;
        Ok(row)
    }

    /// Every task's browser panels, each task's in the order they opened.
    pub fn browsers(&self) -> Result<HashMap<TaskId, Vec<BrowserPanel>>, Error> {
        let mut statement =
            self.db.prepare("SELECT task, panel, url FROM browsers ORDER BY rowid").map_err(Error::internal)?;
        let rows = statement
            .query_map([], |row| {
                Ok((TaskId(row.get(0)?), BrowserPanel { panel: BrowserId(row.get(1)?), url: row.get(2)? }))
            })
            .map_err(Error::internal)?;
        let mut browsers: HashMap<TaskId, Vec<BrowserPanel>> = HashMap::new();
        for row in rows {
            let (task, panel) = row.map_err(Error::internal)?;
            browsers.entry(task).or_default().push(panel);
        }
        Ok(browsers)
    }

    /// Points the task's browser panel at `url`, adding the panel if it is new, and moves the task to a new rev.
    pub fn show_page(&mut self, task: &TaskId, page: &BrowserPanel) -> Result<TaskRow, Error> {
        let tx = self.db.transaction().map_err(Error::internal)?;
        tx.execute(
            "INSERT INTO browsers (task, panel, url) VALUES (?1, ?2, ?3)
             ON CONFLICT(task, panel) DO UPDATE SET url = excluded.url",
            params![task.0, page.panel.0, page.url],
        )
        .map_err(Error::internal)?;
        let rev = bump_rev(&tx)?;
        let row = tx
            .query_row(
                "UPDATE tasks SET rev = ?2 WHERE id = ?1 RETURNING *",
                params![task.0, rev.cast_signed()],
                task_row,
            )
            .map_err(Error::internal)?;
        tx.commit().map_err(Error::internal)?;
        Ok(row)
    }

    /// Replaces the task's previous terminal, if any.
    pub fn record_terminal(&mut self, terminal: &TerminalRow) -> Result<(), Error> {
        self.db
            .execute(
                "INSERT INTO terminals (task, id) VALUES (?1, ?2) ON CONFLICT(task) DO UPDATE SET id = excluded.id",
                params![terminal.task.0, terminal.id.0],
            )
            .map(drop)
            .map_err(Error::internal)
    }

    /// Deletes the terminal rows the host lost and returns the rest; a focused live task left without a terminal
    /// forgets its focus, so its next focus starts its agent again.
    pub fn reattach(&mut self, hosted: &HashSet<PtyId>) -> Result<Vec<TerminalRow>, Error> {
        let tx = self.db.transaction().map_err(Error::internal)?;
        let rows: Vec<TerminalRow> = tx
            .prepare("SELECT id, task FROM terminals")
            .and_then(|mut statement| {
                statement
                    .query_map([], |row| Ok(TerminalRow { id: PtyId(row.get(0)?), task: TaskId(row.get(1)?) }))?
                    .collect()
            })
            .map_err(Error::internal)?;
        let (kept, gone): (Vec<_>, Vec<_>) = rows.into_iter().partition(|row| hosted.contains(&row.id));
        for row in &gone {
            tx.execute("DELETE FROM terminals WHERE id = ?1", [&row.id.0]).map_err(Error::internal)?;
        }
        let live: Vec<String> = TaskState::ALL
            .into_iter()
            .filter(|state| state.is_live())
            .map(|state| format!("'{}'", to_wire(&state)))
            .collect();
        let focused_live = format!(
            "focused_at IS NOT NULL AND state IN ({}) AND id NOT IN (SELECT task FROM terminals)",
            live.join(", ")
        );
        let any: bool = tx
            .query_row(&format!("SELECT EXISTS(SELECT 1 FROM tasks WHERE {focused_live})"), [], |row| row.get(0))
            .map_err(Error::internal)?;
        if any {
            let rev = bump_rev(&tx)?;
            tx.execute(
                &format!("UPDATE tasks SET focused_at = NULL, rev = ?1 WHERE {focused_live}"),
                [rev.cast_signed()],
            )
            .map_err(Error::internal)?;
        }
        tx.commit().map_err(Error::internal)?;
        Ok(kept)
    }
}
