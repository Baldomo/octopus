use std::collections::{HashMap, HashSet};
use std::path::Path;

use octopus_proto::Error;
use octopus_proto::device::{DeviceKey, PublicKey};
use octopus_proto::types::{
    AGENT_PANEL, BrowserId, BrowserPanel, Checkout, Color, Hooks, PtyId, SessionId, TaskId, TaskState, Workspace,
    WorkspaceId, from_wire, to_wire,
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
CREATE TABLE IF NOT EXISTS harnesses (id TEXT PRIMARY KEY, argv TEXT NOT NULL, install TEXT NOT NULL, hooks TEXT NOT NULL);
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
    ended_at INTEGER,
    rev INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS panels (
    id TEXT PRIMARY KEY,
    task TEXT NOT NULL REFERENCES tasks(id),
    name TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS browsers (
    task TEXT NOT NULL REFERENCES tasks(id),
    panel TEXT NOT NULL,
    url TEXT NOT NULL,
    title TEXT NOT NULL DEFAULT '',
    load INTEGER NOT NULL DEFAULT 1,
    place INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (task, panel)
);
CREATE TABLE IF NOT EXISTS device (id INTEGER PRIMARY KEY CHECK (id = 0), pkcs8 BLOB NOT NULL);
CREATE TABLE IF NOT EXISTS clients (name TEXT PRIMARY KEY, key TEXT NOT NULL UNIQUE);
";

/// Columns added to a table after it was first created, which a store gains on its first open by a build that has
/// them.
const ADDED_COLUMNS: &[(&str, &str, &str)] = &[
    ("tasks", "archived_at", "INTEGER"),
    ("tasks", "session", "TEXT"),
    ("clients", "seen_at", "INTEGER"),
    ("harnesses", "start", "TEXT NOT NULL DEFAULT '[]'"),
    ("harnesses", "resume", "TEXT NOT NULL DEFAULT '[]'"),
    ("workspaces", "color", "TEXT NOT NULL DEFAULT 'auto'"),
    ("browsers", "title", "TEXT NOT NULL DEFAULT ''"),
    ("browsers", "load", "INTEGER NOT NULL DEFAULT 1"),
    ("browsers", "place", "INTEGER NOT NULL DEFAULT 0"),
];

/// Columns a store from an earlier build holds and this build drops on its first open.
const DROPPED_COLUMNS: &[(&str, &str)] = &[("tasks", "focused_at")];

/// A task's terminal panel: its agent's, named `agent`, or a shell's.
#[derive(Debug, Clone, PartialEq)]
pub struct PanelRow {
    pub id: PtyId,
    pub task: TaskId,
    pub name: String,
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
    pub session: Option<SessionId>,
    pub directory: String,
    pub branch: Option<String>,
    pub ended_at: Option<i64>,
    pub archived_at: Option<i64>,
    pub rev: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Harness {
    pub argv: Vec<String>,
    pub start: Vec<String>,
    pub resume: Vec<String>,
    pub install: String,
    pub hooks: Hooks,
}

fn owner_only(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    std::fs::OpenOptions::new().create(true).append(true).mode(0o600).open(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

pub struct Store {
    db: Connection,
}

fn bump_rev(tx: &Transaction) -> Result<u64, Error> {
    tx.query_row("UPDATE meta SET value = value + 1 WHERE key = 'rev' RETURNING value", [], |row| row.get::<_, i64>(0))
        .map(i64::cast_unsigned)
        .map_err(Error::internal)
}

/// Moves the task to a new rev and commits.
fn revise_task(tx: Transaction, task: &TaskId) -> Result<TaskRow, Error> {
    let rev = bump_rev(&tx)?;
    let row = tx
        .query_row("UPDATE tasks SET rev = ?2 WHERE id = ?1 RETURNING *", params![task.0, rev.cast_signed()], task_row)
        .map_err(Error::internal)?;
    tx.commit().map_err(Error::internal)?;
    Ok(row)
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
        session: row.get::<_, Option<String>>("session")?.map(SessionId),
        directory: row.get("directory")?,
        branch: row.get("branch")?,
        ended_at: row.get("ended_at")?,
        archived_at: row.get("archived_at")?,
        rev: row.get::<_, i64>("rev")?.cast_unsigned(),
    })
}

/// The column's declared type, when the table has it.
fn column_type(db: &Connection, table: &str, column: &str) -> rusqlite::Result<Option<String>> {
    db.query_row("SELECT type FROM pragma_table_info(?1) WHERE name = ?2", [table, column], |row| row.get(0)).optional()
}

fn has_column(db: &Connection, table: &str, column: &str) -> rusqlite::Result<bool> {
    column_type(db, table, column).map(|kind| kind.is_some())
}

fn migrate_columns(db: &Connection) -> rusqlite::Result<()> {
    for (table, column, kind) in ADDED_COLUMNS {
        if !has_column(db, table, column)? {
            db.execute(&format!("ALTER TABLE {table} ADD COLUMN {column} {kind}"), [])?;
        }
    }
    for (table, column) in DROPPED_COLUMNS {
        if has_column(db, table, column)? {
            db.execute(&format!("ALTER TABLE {table} DROP COLUMN {column}"), [])?;
        }
    }
    Ok(())
}

/// A store from a build whose harnesses declared hooks as a flag declares none for each of them.
fn name_hooks(db: &Connection) -> rusqlite::Result<()> {
    if column_type(db, "harnesses", "hooks")?.as_deref() == Some("INTEGER") {
        let tx = db.unchecked_transaction()?;
        tx.execute("ALTER TABLE harnesses DROP COLUMN hooks", [])?;
        tx.execute("ALTER TABLE harnesses ADD COLUMN hooks TEXT NOT NULL DEFAULT 'none'", [])?;
        tx.commit()?;
    }
    Ok(())
}

const CLAUDE_CODE: &str = "claude-code";

impl Harness {
    fn claude_code() -> Self {
        let session = crate::SESSION.to_string();
        Self {
            argv: vec!["claude".into()],
            start: vec!["--session-id".into(), session.clone()],
            resume: vec!["--resume".into(), session],
            install: "npm install -g @anthropic-ai/claude-code".into(),
            hooks: Hooks::ClaudeCode,
        }
    }
}

fn put_harness(db: &Connection, id: &str, harness: &Harness) -> rusqlite::Result<()> {
    let json = |args: &Vec<String>| serde_json::to_string(args).expect("a list of strings is JSON");
    db.execute(
        "INSERT INTO harnesses (id, argv, start, resume, install, hooks) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(id) DO UPDATE SET argv = excluded.argv, start = excluded.start, resume = excluded.resume,
         install = excluded.install, hooks = excluded.hooks",
        params![
            id,
            json(&harness.argv),
            json(&harness.start),
            json(&harness.resume),
            harness.install,
            to_wire(&harness.hooks)
        ],
    )
    .map(drop)
}

/// Once per store, marked in `meta`, so a later `harness.set` stands: each harness whose argv runs the `claude`
/// program reports through Claude Code hooks, and a store without a `claude-code` harness gains one.
fn install_claude_code(db: &Connection) -> rusqlite::Result<()> {
    let tx = db.unchecked_transaction()?;
    if tx.execute("INSERT OR IGNORE INTO meta (key, value) VALUES ('claude_code_installed', 1)", [])? == 0 {
        return Ok(());
    }
    tx.execute(
        "UPDATE harnesses SET hooks = ?1
         WHERE json_extract(argv, '$[0]') = 'claude' OR json_extract(argv, '$[0]') LIKE '%/claude'",
        [to_wire(&Hooks::ClaudeCode)],
    )?;
    let present: bool =
        tx.query_row("SELECT EXISTS(SELECT 1 FROM harnesses WHERE id = ?1)", [CLAUDE_CODE], |row| row.get(0))?;
    if !present {
        put_harness(&tx, CLAUDE_CODE, &Harness::claude_code())?;
    }
    tx.commit()
}

/// A store from a build that kept one agent terminal per task gains each as the task's agent panel.
fn adopt_agent_terminals(db: &Connection) -> rusqlite::Result<()> {
    let present: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'terminals')",
        [],
        |row| row.get(0),
    )?;
    if present {
        let tx = db.unchecked_transaction()?;
        tx.execute("INSERT INTO panels (id, task, name) SELECT id, task, ?1 FROM terminals", [AGENT_PANEL])?;
        tx.execute("DROP TABLE terminals", [])?;
        tx.commit()?;
    }
    Ok(())
}

fn workspace_row(row: &Row) -> rusqlite::Result<Workspace> {
    Ok(Workspace {
        id: WorkspaceId(row.get("id")?),
        name: row.get("name")?,
        root: row.get("root")?,
        color: wire_column(row, "color")?,
    })
}

impl Store {
    /// The store holds the server's private key, so only its owner may read it.
    pub fn open(path: &Path) -> Result<Self, Error> {
        owner_only(path).map_err(|error| Error::internal(format!("{}: {error}", path.display())))?;
        let db = Connection::open(path).map_err(Error::internal)?;
        db.execute_batch(SCHEMA).map_err(Error::internal)?;
        migrate_columns(&db).map_err(Error::internal)?;
        name_hooks(&db).map_err(Error::internal)?;
        install_claude_code(&db).map_err(Error::internal)?;
        adopt_agent_terminals(&db).map_err(Error::internal)?;
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

    /// A name pinned to another device stays its until `pairing.forget` lets it go.
    pub fn refuse_claim(&self, name: &str, key: &PublicKey) -> Result<(), Error> {
        let holder: Option<String> = self
            .db
            .query_row("SELECT key FROM clients WHERE name = ?1", [name], |row| row.get(0))
            .optional()
            .map_err(Error::internal)?;
        match holder {
            Some(holder) if holder != key.to_string() => Err(Error::conflict(format!(
                "the client name {name} is paired to another device; rename this device, or `pairing forget` the other \
                 one"
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

    /// Unpins the named client and returns the key it was pinned to.
    pub fn forget_client(&mut self, name: &str) -> Result<PublicKey, Error> {
        self.db
            .query_row("DELETE FROM clients WHERE name = ?1 RETURNING key", [name], |row| wire_column(row, "key"))
            .optional()
            .map_err(Error::internal)?
            .ok_or_else(|| Error::not_found(format!("a paired client named {name}")))
    }

    /// Every pinned client's key and name, with when one of its connections last opened or closed.
    pub fn clients(&self) -> Result<Vec<(PublicKey, String, Option<i64>)>, Error> {
        let mut statement =
            self.db.prepare("SELECT key, name, seen_at FROM clients ORDER BY name").map_err(Error::internal)?;
        let rows = statement
            .query_map([], |row| Ok((wire_column(row, "key")?, row.get("name")?, row.get("seen_at")?)))
            .map_err(Error::internal)?;
        rows.collect::<Result<_, _>>().map_err(Error::internal)
    }

    pub fn client_seen(&self, key: &PublicKey, at: i64) -> Result<(), Error> {
        self.db
            .execute("UPDATE clients SET seen_at = ?2 WHERE key = ?1", params![key.to_string(), at])
            .map(drop)
            .map_err(Error::internal)
    }

    pub fn client_pinned_to(&self, key: &PublicKey) -> Result<Option<String>, Error> {
        self.db
            .query_row("SELECT name FROM clients WHERE key = ?1", [key.to_string()], |row| row.get(0))
            .optional()
            .map_err(Error::internal)
    }

    pub fn rename_client(&self, key: &PublicKey, name: &str) -> Result<(), Error> {
        self.db
            .execute("UPDATE clients SET name = ?2 WHERE key = ?1", params![key.to_string(), name])
            .map(drop)
            .map_err(Error::internal)
    }

    pub fn rev(&self) -> Result<u64, Error> {
        self.db
            .query_row("SELECT value FROM meta WHERE key = 'rev'", [], |row| row.get::<_, i64>(0))
            .map(i64::cast_unsigned)
            .map_err(Error::internal)
    }

    pub fn add_workspace(&mut self, name: &str, root: &str, color: Color) -> Result<(Workspace, u64), Error> {
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
        let workspace = Workspace { id: WorkspaceId::random(), name: name.to_string(), root: root.to_string(), color };
        tx.execute(
            "INSERT INTO workspaces (id, name, root, color) VALUES (?1, ?2, ?3, ?4)",
            params![workspace.id.0, workspace.name, workspace.root, to_wire(&color)],
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

    pub fn rename_workspace(&mut self, id: &WorkspaceId, name: &str) -> Result<(Workspace, u64), Error> {
        let tx = self.db.transaction().map_err(Error::internal)?;
        let taken: bool = tx
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM workspaces WHERE name = ?1 AND id != ?2)",
                params![name, id.0],
                |row| row.get(0),
            )
            .map_err(Error::internal)?;
        if taken {
            return Err(Error::conflict(format!("a workspace named {name} exists")));
        }
        let workspace = tx
            .query_row("UPDATE workspaces SET name = ?2 WHERE id = ?1 RETURNING *", params![id.0, name], workspace_row)
            .optional()
            .map_err(Error::internal)?
            .ok_or_else(|| Error::not_found(id))?;
        let rev = bump_rev(&tx)?;
        tx.commit().map_err(Error::internal)?;
        Ok((workspace, rev))
    }

    pub fn color_workspace(&mut self, id: &WorkspaceId, color: Color) -> Result<(Workspace, u64), Error> {
        let tx = self.db.transaction().map_err(Error::internal)?;
        let workspace = tx
            .query_row(
                "UPDATE workspaces SET color = ?2 WHERE id = ?1 RETURNING *",
                params![id.0, to_wire(&color)],
                workspace_row,
            )
            .optional()
            .map_err(Error::internal)?
            .ok_or_else(|| Error::not_found(id))?;
        let rev = bump_rev(&tx)?;
        tx.commit().map_err(Error::internal)?;
        Ok((workspace, rev))
    }

    pub fn workspace_tasks(&self, id: &WorkspaceId) -> Result<Vec<TaskRow>, Error> {
        let mut statement =
            self.db.prepare("SELECT * FROM tasks WHERE workspace = ?1 ORDER BY rowid").map_err(Error::internal)?;
        let rows = statement.query_map([&id.0], task_row).map_err(Error::internal)?;
        rows.collect::<Result<_, _>>().map_err(Error::internal)
    }

    /// Deletes the workspace with its tasks and their terminal and browser rows. Each task of another workspace
    /// whose parent goes becomes a root task at a rev of its own, below the removal's.
    pub fn remove_workspace(&mut self, id: &WorkspaceId) -> Result<(Vec<TaskRow>, u64), Error> {
        let tx = self.db.transaction().map_err(Error::internal)?;
        let orphans: Vec<String> = tx
            .prepare(
                "SELECT id FROM tasks WHERE workspace != ?1
                 AND parent IN (SELECT id FROM tasks WHERE workspace = ?1) ORDER BY rowid",
            )
            .and_then(|mut statement| statement.query_map([&id.0], |row| row.get(0))?.collect())
            .map_err(Error::internal)?;
        let mut rerooted = Vec::new();
        for orphan in orphans {
            let rev = bump_rev(&tx)?;
            let row = tx
                .query_row(
                    "UPDATE tasks SET parent = NULL, rev = ?2 WHERE id = ?1 RETURNING *",
                    params![orphan, rev.cast_signed()],
                    task_row,
                )
                .map_err(Error::internal)?;
            rerooted.push(row);
        }
        let in_workspace = "task IN (SELECT id FROM tasks WHERE workspace = ?1)";
        for table in ["panels", "browsers"] {
            tx.execute(&format!("DELETE FROM {table} WHERE {in_workspace}"), [&id.0]).map_err(Error::internal)?;
        }
        tx.execute("DELETE FROM tasks WHERE workspace = ?1", [&id.0]).map_err(Error::internal)?;
        tx.execute("DELETE FROM workspaces WHERE id = ?1", [&id.0]).map_err(Error::internal)?;
        let rev = bump_rev(&tx)?;
        tx.commit().map_err(Error::internal)?;
        Ok((rerooted, rev))
    }

    pub fn set_harness(&mut self, id: &str, harness: &Harness) -> Result<u64, Error> {
        let tx = self.db.transaction().map_err(Error::internal)?;
        put_harness(&tx, id, harness).map_err(Error::internal)?;
        let rev = bump_rev(&tx)?;
        tx.commit().map_err(Error::internal)?;
        Ok(rev)
    }

    pub fn harness(&self, id: &str) -> Result<Option<Harness>, Error> {
        let row: Option<([String; 3], String, Hooks)> = self
            .db
            .query_row("SELECT argv, start, resume, install, hooks FROM harnesses WHERE id = ?1", [id], |row| {
                Ok(([row.get(0)?, row.get(1)?, row.get(2)?], row.get(3)?, wire_column(row, "hooks")?))
            })
            .optional()
            .map_err(Error::internal)?;
        row.map(|([argv, start, resume], install, hooks)| {
            let args = |json: &str| serde_json::from_str(json).map_err(Error::internal);
            Ok(Harness { argv: args(&argv)?, start: args(&start)?, resume: args(&resume)?, install, hooks })
        })
        .transpose()
    }

    pub fn create_task(&mut self, mut row: TaskRow) -> Result<TaskRow, Error> {
        let tx = self.db.transaction().map_err(Error::internal)?;
        row.rev = bump_rev(&tx)?;
        tx.execute(
            "INSERT INTO tasks (id, workspace, parent, title, state, checkout, harness, session, directory, branch, ended_at, archived_at, rev)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                row.id.0,
                row.workspace.0,
                row.parent.as_ref().map(|parent| &parent.0),
                row.title,
                to_wire(&row.state),
                to_wire(&row.checkout),
                row.harness,
                row.session.as_ref().map(|session| &session.0),
                row.directory,
                row.branch,
                row.ended_at,
                row.archived_at,
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
            "UPDATE tasks SET title = ?2, state = ?3, session = ?4, ended_at = ?5, archived_at = ?6, rev = ?7, parent = ?8
             WHERE id = ?1",
            params![
                row.id.0,
                row.title,
                to_wire(&row.state),
                row.session.as_ref().map(|session| &session.0),
                row.ended_at,
                row.archived_at,
                row.rev.cast_signed(),
                row.parent.as_ref().map(|parent| &parent.0)
            ],
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

    /// Every task's browser panels, each task's in its order.
    pub fn browsers(&self) -> Result<HashMap<TaskId, Vec<BrowserPanel>>, Error> {
        let mut statement = self
            .db
            .prepare("SELECT task, panel, url, title, load FROM browsers ORDER BY place, rowid")
            .map_err(Error::internal)?;
        let rows = statement
            .query_map([], |row| {
                let panel = BrowserPanel {
                    panel: BrowserId(row.get(1)?),
                    url: row.get(2)?,
                    title: row.get(3)?,
                    load: row.get::<_, i64>(4)?.cast_unsigned(),
                };
                Ok((TaskId(row.get(0)?), panel))
            })
            .map_err(Error::internal)?;
        let mut browsers: HashMap<TaskId, Vec<BrowserPanel>> = HashMap::new();
        for row in rows {
            let (task, panel) = row.map_err(Error::internal)?;
            browsers.entry(task).or_default().push(panel);
        }
        Ok(browsers)
    }

    /// Saves the task's browser panel, adding a new one after the task's others, and moves the task to a new rev.
    pub fn save_browser(&mut self, task: &TaskId, browser: &BrowserPanel) -> Result<TaskRow, Error> {
        let tx = self.db.transaction().map_err(Error::internal)?;
        tx.execute(
            "INSERT INTO browsers (task, panel, url, title, load, place)
             VALUES (?1, ?2, ?3, ?4, ?5, (SELECT COALESCE(MAX(place) + 1, 0) FROM browsers WHERE task = ?1))
             ON CONFLICT(task, panel) DO UPDATE SET url = excluded.url, title = excluded.title, load = excluded.load",
            params![task.0, browser.panel.0, browser.url, browser.title, browser.load.cast_signed()],
        )
        .map_err(Error::internal)?;
        revise_task(tx, task)
    }

    /// Puts the task's browser panels in the order of `panels`, and moves the task to a new rev.
    pub fn order_browsers(&mut self, task: &TaskId, panels: &[BrowserPanel]) -> Result<TaskRow, Error> {
        let tx = self.db.transaction().map_err(Error::internal)?;
        for (place, browser) in (0_i64..).zip(panels) {
            tx.execute(
                "UPDATE browsers SET place = ?3 WHERE task = ?1 AND panel = ?2",
                params![task.0, browser.panel.0, place],
            )
            .map_err(Error::internal)?;
        }
        revise_task(tx, task)
    }

    /// Removes the task's browser panel and moves the task to a new rev.
    pub fn close_browser(&mut self, task: &TaskId, panel: &BrowserId) -> Result<TaskRow, Error> {
        let tx = self.db.transaction().map_err(Error::internal)?;
        tx.execute("DELETE FROM browsers WHERE task = ?1 AND panel = ?2", params![task.0, panel.0])
            .map_err(Error::internal)?;
        revise_task(tx, task)
    }

    /// An agent panel replaces the task's previous one; a shell panel joins the end of the task's list.
    pub fn record_panel(&mut self, panel: &PanelRow) -> Result<(), Error> {
        let tx = self.db.transaction().map_err(Error::internal)?;
        if panel.name == AGENT_PANEL {
            tx.execute("DELETE FROM panels WHERE task = ?1 AND name = ?2", params![panel.task.0, AGENT_PANEL])
                .map_err(Error::internal)?;
        }
        tx.execute(
            "INSERT INTO panels (id, task, name) VALUES (?1, ?2, ?3)",
            params![panel.id.0, panel.task.0, panel.name],
        )
        .map_err(Error::internal)?;
        tx.commit().map_err(Error::internal)
    }

    pub fn forget_panel(&mut self, id: &PtyId) -> Result<(), Error> {
        self.db.execute("DELETE FROM panels WHERE id = ?1", [&id.0]).map(drop).map_err(Error::internal)
    }

    /// Deletes the panel rows the host lost and returns the rest, each task's in list order.
    pub fn reattach(&mut self, hosted: &HashSet<PtyId>) -> Result<Vec<PanelRow>, Error> {
        let tx = self.db.transaction().map_err(Error::internal)?;
        let rows: Vec<PanelRow> = tx
            .prepare("SELECT id, task, name FROM panels ORDER BY rowid")
            .and_then(|mut statement| {
                statement
                    .query_map([], |row| {
                        Ok(PanelRow { id: PtyId(row.get(0)?), task: TaskId(row.get(1)?), name: row.get(2)? })
                    })?
                    .collect()
            })
            .map_err(Error::internal)?;
        let (kept, gone): (Vec<_>, Vec<_>) = rows.into_iter().partition(|row| hosted.contains(&row.id));
        for row in &gone {
            tx.execute("DELETE FROM panels WHERE id = ?1", [&row.id.0]).map_err(Error::internal)?;
        }
        tx.commit().map_err(Error::internal)?;
        Ok(kept)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_store_from_an_earlier_build_gains_and_drops_the_columns_changed_since() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("store.db");
        let earlier = Connection::open(&path).expect("open");
        earlier
            .execute_batch(
                "CREATE TABLE tasks (id TEXT PRIMARY KEY, workspace TEXT NOT NULL, parent TEXT, title TEXT NOT NULL,
                 state TEXT NOT NULL, checkout TEXT NOT NULL, harness TEXT NOT NULL, directory TEXT NOT NULL,
                 branch TEXT, focused_at INTEGER, ended_at INTEGER, rev INTEGER NOT NULL);
                 INSERT INTO tasks VALUES ('tsk_1', 'wsp_1', NULL, 't', 'pinned', 'root', 'h', '/w', NULL, NULL, NULL, 1);
                 CREATE TABLE terminals (task TEXT PRIMARY KEY, id TEXT NOT NULL UNIQUE);
                 INSERT INTO terminals VALUES ('tsk_1', 'pty_1');
                 CREATE TABLE clients (name TEXT PRIMARY KEY, key TEXT NOT NULL UNIQUE);
                 CREATE TABLE harnesses (id TEXT PRIMARY KEY, argv TEXT NOT NULL, install TEXT NOT NULL,
                 hooks INTEGER NOT NULL);
                 INSERT INTO harnesses VALUES ('h', '[\"h\"]', 'true', 1);
                 INSERT INTO harnesses VALUES ('claude-code', '[\"/opt/bin/claude\"]', 'true', 0);",
            )
            .expect("an earlier schema");
        drop(earlier);
        let mut store = Store::open(&path).expect("open the earlier store");
        let task = store.task(&TaskId("tsk_1".into())).expect("its task");
        assert_eq!((task.archived_at, task.session), (None, None));
        assert!(!has_column(&store.db, "tasks", "focused_at").expect("table info"), "the dropped column goes");
        assert_eq!(store.clients().expect("clients"), []);
        assert_eq!(store.harness("h").expect("its harness").map(|harness| harness.hooks), Some(Hooks::None));
        let claude = store.harness("claude-code").expect("its harness").expect("kept");
        assert_eq!(
            (claude.argv, claude.install, claude.hooks),
            (vec!["/opt/bin/claude".into()], "true".into(), Hooks::ClaudeCode)
        );
        let agent = PanelRow { id: PtyId("pty_1".into()), task: TaskId("tsk_1".into()), name: AGENT_PANEL.into() };
        assert_eq!(store.reattach(&HashSet::from([agent.id.clone()])).expect("reattach"), [agent]);
    }

    #[test]
    fn a_task_keeps_its_browser_panels_in_the_order_they_moved_to_and_a_new_one_joins_the_end() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("store.db");
        let mut store = Store::open(&path).expect("a fresh store");
        let (workspace, _) = store.add_workspace("w", "/w", Color::Auto).expect("a workspace");
        store
            .db
            .execute(
                "INSERT INTO tasks (id, workspace, title, state, checkout, harness, directory, rev)
                 VALUES ('tsk_1', ?1, 't', 'pinned', 'root', 'h', '/w', 1)",
                [workspace.id.0],
            )
            .expect("a task");
        let task = TaskId("tsk_1".into());
        let panel = |id: &str| BrowserPanel {
            panel: BrowserId(id.into()),
            url: format!("http://localhost/{id}"),
            title: String::new(),
            load: 1,
        };
        for id in ["a", "b", "c"] {
            store.save_browser(&task, &panel(id)).expect("save");
        }
        store.order_browsers(&task, &[panel("c"), panel("a"), panel("b")]).expect("order");
        store.save_browser(&task, &panel("a")).expect("save again");
        store.save_browser(&task, &panel("d")).expect("save a new one");
        drop(store);
        let browsers = Store::open(&path).expect("reopen").browsers().expect("browsers");
        assert_eq!(browsers[&task], [panel("c"), panel("a"), panel("b"), panel("d")]);
    }

    #[test]
    fn a_fresh_store_runs_claude_code_with_hooks_until_the_user_sets_it_otherwise() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("store.db");
        let mut store = Store::open(&path).expect("a fresh store");
        let claude_code = Harness {
            argv: vec!["claude".into()],
            start: vec!["--session-id".into(), "{session}".into()],
            resume: vec!["--resume".into(), "{session}".into()],
            install: "npm install -g @anthropic-ai/claude-code".into(),
            hooks: Hooks::ClaudeCode,
        };
        assert_eq!(store.harness("claude-code").expect("harness"), Some(claude_code.clone()));
        let without_hooks = Harness { hooks: Hooks::None, ..claude_code };
        store.set_harness("claude-code", &without_hooks).expect("harness.set");
        drop(store);
        let store = Store::open(&path).expect("reopen");
        assert_eq!(store.harness("claude-code").expect("harness"), Some(without_hooks));
    }
}
