use std::collections::{HashMap, HashSet};
use std::path::Path;

use octopus_proto::Error;
use octopus_proto::account::{Account, AccountId};
use octopus_proto::device::{DeviceKey, PublicKey};
use octopus_proto::settings::Settings;
use octopus_proto::types::{
    AGENT_PANEL, BrowserId, BrowserPanel, Color, Mode, PtyId, Session, SessionId, TaskId, TaskState, Workspace,
    WorkspaceId, from_wire, to_wire,
};
use rusqlite::types::Type;
use rusqlite::{Connection, OptionalExtension, Row, Transaction, params};
use serde::de::DeserializeOwned;

use crate::harness;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value INTEGER NOT NULL);
INSERT OR IGNORE INTO meta (key, value) VALUES ('rev', 0);
CREATE TABLE IF NOT EXISTS workspaces (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    root TEXT NOT NULL,
    color TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS tasks (
    id TEXT PRIMARY KEY,
    workspace TEXT NOT NULL REFERENCES workspaces(id),
    parent TEXT REFERENCES tasks(id),
    title TEXT NOT NULL,
    state TEXT NOT NULL,
    folder TEXT NOT NULL,
    mode TEXT NOT NULL,
    directory TEXT NOT NULL,
    worktree TEXT,
    branch TEXT,
    harness TEXT NOT NULL,
    session TEXT NOT NULL,
    session_started INTEGER NOT NULL,
    ended_at INTEGER,
    archived_at INTEGER,
    rev INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS harnesses (
    id TEXT PRIMARY KEY,
    argv TEXT NOT NULL,
    start TEXT NOT NULL,
    resume TEXT NOT NULL,
    install TEXT NOT NULL
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
    title TEXT NOT NULL,
    load INTEGER NOT NULL,
    place INTEGER NOT NULL,
    PRIMARY KEY (task, panel)
);
CREATE TABLE IF NOT EXISTS device (id INTEGER PRIMARY KEY CHECK (id = 0), pkcs8 BLOB NOT NULL);
CREATE TABLE IF NOT EXISTS clients (name TEXT PRIMARY KEY, key TEXT NOT NULL UNIQUE, seen_at INTEGER);
CREATE TABLE IF NOT EXISTS accounts (id TEXT PRIMARY KEY, log TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS settings (account TEXT PRIMARY KEY, sets TEXT NOT NULL);
CREATE TABLE IF NOT EXISTS owner (id INTEGER PRIMARY KEY CHECK (id = 0), account TEXT NOT NULL);
";

const DROP_HARNESS_HOOKS: &str = "ALTER TABLE harnesses DROP COLUMN hooks";

const INSERT_TASK: &str = "INSERT INTO tasks (id, workspace, parent, title, state, folder, mode, directory, worktree,
    branch, harness, session, session_started, ended_at, archived_at, rev)
    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)";

/// Lays the store out in `SCHEMA`, dropping the hooks of its harnesses.
fn lay_out(tx: &Transaction) -> Result<(), Error> {
    tx.execute_batch(SCHEMA).map_err(Error::internal)?;
    let hooks: bool = tx
        .query_row("SELECT EXISTS(SELECT 1 FROM pragma_table_info('harnesses') WHERE name = 'hooks')", [], |row| {
            row.get(0)
        })
        .map_err(Error::internal)?;
    if hooks {
        tx.execute_batch(DROP_HARNESS_HOOKS).map_err(Error::internal)?;
    }
    Ok(())
}

fn select<T, C: FromIterator<T>>(
    db: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
    row: impl FnMut(&Row) -> rusqlite::Result<T>,
) -> Result<C, Error> {
    db.prepare(sql).and_then(|mut statement| statement.query_map(params, row)?.collect()).map_err(Error::internal)
}

fn put_account(db: &Connection, account: &Account) -> Result<(), Error> {
    let log = serde_json::to_string(account).map_err(Error::internal)?;
    db.execute(
        "INSERT INTO accounts (id, log) VALUES (?1, ?2) ON CONFLICT(id) DO UPDATE SET log = excluded.log",
        params![account.id().to_string(), log],
    )
    .map(drop)
    .map_err(Error::internal)
}

/// A task's terminal panel: its agent's, named `agent`, or a shell's.
#[derive(Debug, Clone, PartialEq)]
pub struct PanelRow {
    pub id: PtyId,
    pub task: TaskId,
    pub name: String,
}

/// A worktree or copy task's worktree: where it sits and its branch.
#[derive(Debug, Clone, PartialEq)]
pub struct Worktree {
    pub path: String,
    pub branch: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TaskRow {
    pub id: TaskId,
    pub workspace: WorkspaceId,
    pub parent: Option<TaskId>,
    pub title: String,
    pub state: TaskState,
    pub folder: String,
    pub mode: Mode,
    pub directory: String,
    pub worktree: Option<Worktree>,
    pub harness: String,
    pub session: Session,
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

fn commit_rev(tx: Transaction) -> Result<u64, Error> {
    let rev = bump_rev(&tx)?;
    tx.commit().map_err(Error::internal)?;
    Ok(rev)
}

/// Moves the task to a new rev and commits.
fn revise_task(tx: Transaction, task: &TaskId) -> Result<TaskRow, Error> {
    let rev = bump_rev(&tx)?;
    let row = tx
        .query_row("UPDATE tasks SET rev = ?2 WHERE id = ?1 RETURNING *", params![task.0, rev.cast_signed()], task_row)
        .optional()
        .map_err(Error::internal)?
        .ok_or_else(|| Error::not_found(task))?;
    tx.commit().map_err(Error::internal)?;
    Ok(row)
}

fn wire_column<T: DeserializeOwned>(row: &Row, column: &str) -> rusqlite::Result<T> {
    let text: String = row.get(column)?;
    from_wire(&text).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(0, Type::Text, format!("{column} = {text:?}: {error}").into())
    })
}

/// A worktree or copy task holds a worktree, and an `in_place` task none.
fn worktree(row: &Row, mode: Mode) -> rusqlite::Result<Option<Worktree>> {
    match (mode, row.get::<_, Option<String>>("worktree")?, row.get::<_, Option<String>>("branch")?) {
        (Mode::InPlace, None, None) => Ok(None),
        (Mode::Worktree | Mode::Copy, Some(path), Some(branch)) => Ok(Some(Worktree { path, branch })),
        (mode, path, branch) => Err(rusqlite::Error::FromSqlConversionFailure(
            0,
            Type::Text,
            format!("mode {} with worktree {path:?} and branch {branch:?}", to_wire(&mode)).into(),
        )),
    }
}

fn task_row(row: &Row) -> rusqlite::Result<TaskRow> {
    let mode = wire_column(row, "mode")?;
    Ok(TaskRow {
        id: TaskId(row.get("id")?),
        workspace: WorkspaceId(row.get("workspace")?),
        parent: row.get::<_, Option<String>>("parent")?.map(TaskId),
        title: row.get("title")?,
        state: wire_column(row, "state")?,
        folder: row.get("folder")?,
        mode,
        directory: row.get("directory")?,
        worktree: worktree(row, mode)?,
        harness: row.get("harness")?,
        session: Session { id: SessionId(row.get("session")?), started: row.get("session_started")? },
        ended_at: row.get("ended_at")?,
        archived_at: row.get("archived_at")?,
        rev: row.get::<_, i64>("rev")?.cast_unsigned(),
    })
}

const INSERT_HARNESS: &str = "INSERT INTO harnesses (id, argv, start, resume, install) VALUES (?1, ?2, ?3, ?4, ?5)";

fn harness_values(id: &str, harness: &Harness) -> [String; 5] {
    let json = |args: &Vec<String>| serde_json::to_string(args).expect("a list of strings is JSON");
    [id.to_string(), json(&harness.argv), json(&harness.start), json(&harness.resume), harness.install.clone()]
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
    /// The store holds the server's private key, so only its owner may read it. A store whose tasks lack a column
    /// this server writes fails to open, naming the store, and stays as it was.
    pub fn open(path: &Path) -> Result<Self, Error> {
        owner_only(path).map_err(|error| Error::internal(format!("{}: {error}", path.display())))?;
        let mut db = Connection::open(path).map_err(Error::internal)?;
        db.execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;").map_err(Error::internal)?;
        let tx = db.transaction().map_err(Error::internal)?;
        let unfit = |error| Error::internal(format!("{}: this server cannot open it: {error}", path.display()));
        lay_out(&tx).map_err(unfit)?;
        tx.prepare(INSERT_TASK).map(drop).map_err(|error| unfit(Error::internal(error)))?;
        // A stored `claude-code` harness stands as `harness.set` left it.
        let seed = format!("{INSERT_HARNESS} ON CONFLICT(id) DO NOTHING");
        tx.execute(&seed, harness_values(harness::CLAUDE_CODE, &harness::claude_code())).map_err(Error::internal)?;
        tx.commit().map_err(Error::internal)?;
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

    /// A name recorded for another device stays its until that device's account is forgotten.
    pub fn refuse_claim(&self, name: &str, key: &PublicKey) -> Result<(), Error> {
        let holder: Option<String> = self
            .db
            .query_row("SELECT key FROM clients WHERE name = ?1", [name], |row| row.get(0))
            .optional()
            .map_err(Error::internal)?;
        match holder {
            Some(holder) if holder != key.to_string() => Err(Error::conflict(format!(
                "the client name {name} belongs to another device; rename this device, or `pairing forget` the other \
                 one's account"
            ))),
            _ => Ok(()),
        }
    }

    /// Records the name the device gives, keeping when it was last seen.
    pub fn name_device(&mut self, name: &str, key: &PublicKey) -> Result<(), Error> {
        self.db
            .execute(
                "INSERT INTO clients (name, key) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET name = excluded.name",
                params![name, key.to_string()],
            )
            .map(drop)
            .map_err(Error::internal)
    }

    pub fn forget_devices(&mut self, keys: &[PublicKey]) -> Result<(), Error> {
        let tx = self.db.transaction().map_err(Error::internal)?;
        for key in keys {
            tx.execute("DELETE FROM clients WHERE key = ?1", [key.to_string()]).map_err(Error::internal)?;
        }
        tx.commit().map_err(Error::internal)
    }

    /// Each recorded device's name and when one of its connections last opened or closed.
    pub fn devices(&self) -> Result<HashMap<PublicKey, (String, Option<i64>)>, Error> {
        select(&self.db, "SELECT key, name, seen_at FROM clients", [], |row| {
            Ok((wire_column(row, "key")?, (row.get("name")?, row.get("seen_at")?)))
        })
    }

    pub fn device_name(&self, key: &PublicKey) -> Result<Option<String>, Error> {
        self.db
            .query_row("SELECT name FROM clients WHERE key = ?1", [key.to_string()], |row| row.get(0))
            .optional()
            .map_err(Error::internal)
    }

    pub fn client_seen(&self, key: &PublicKey, at: i64) -> Result<(), Error> {
        self.db
            .execute("UPDATE clients SET seen_at = ?2 WHERE key = ?1", params![key.to_string(), at])
            .map(drop)
            .map_err(Error::internal)
    }

    /// Each trusted account's log and its settings, empty for an account no device set any of.
    pub fn accounts(&self) -> Result<Vec<(Account, Settings)>, Error> {
        let sql = "SELECT log, sets FROM accounts LEFT JOIN settings ON settings.account = accounts.id";
        let rows: Vec<(String, Option<String>)> = select(&self.db, sql, [], |row| Ok((row.get(0)?, row.get(1)?)))?;
        rows.iter()
            .map(|(log, sets)| {
                let account: Account = serde_json::from_str(log).map_err(Error::internal)?;
                let settings = match sets {
                    Some(sets) => serde_json::from_str(sets).map_err(Error::internal)?,
                    None => Settings::new(account.id()),
                };
                Ok((account, settings))
            })
            .collect()
    }

    pub fn keep_account(&mut self, account: &Account, settings: &Settings) -> Result<(), Error> {
        let tx = self.db.transaction().map_err(Error::internal)?;
        put_account(&tx, account)?;
        tx.execute(
            "INSERT INTO settings (account, sets) VALUES (?1, ?2) ON CONFLICT(account) DO UPDATE SET sets = excluded.sets",
            params![settings.account.to_string(), serde_json::to_string(settings).map_err(Error::internal)?],
        )
        .map_err(Error::internal)?;
        tx.commit().map_err(Error::internal)
    }

    pub fn forget_account(&mut self, id: &AccountId) -> Result<(), Error> {
        let tx = self.db.transaction().map_err(Error::internal)?;
        tx.execute("DELETE FROM accounts WHERE id = ?1", [id.to_string()]).map_err(Error::internal)?;
        tx.execute("DELETE FROM settings WHERE account = ?1", [id.to_string()]).map_err(Error::internal)?;
        tx.commit().map_err(Error::internal)
    }

    /// The account a client on this machine sent last.
    pub fn owner(&self) -> Result<Option<AccountId>, Error> {
        let owner: Option<String> =
            self.db.query_row("SELECT account FROM owner", [], |row| row.get(0)).optional().map_err(Error::internal)?;
        owner.map(|id| id.parse().map_err(Error::internal)).transpose()
    }

    pub fn set_owner(&self, id: &AccountId) -> Result<(), Error> {
        self.db
            .execute(
                "INSERT INTO owner (id, account) VALUES (0, ?1) ON CONFLICT(id) DO UPDATE SET account = excluded.account",
                [id.to_string()],
            )
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
            .query_row("SELECT EXISTS(SELECT 1 FROM workspaces WHERE name = ?1)", [name], |row| row.get(0))
            .map_err(Error::internal)?;
        if taken {
            return Err(Error::conflict(format!("a workspace named {name} exists")));
        }
        let workspace = Workspace { id: WorkspaceId::random(), name: name.to_string(), root: root.to_string(), color };
        tx.execute(
            "INSERT INTO workspaces (id, name, root, color) VALUES (?1, ?2, ?3, ?4)",
            params![workspace.id.0, workspace.name, workspace.root, to_wire(&color)],
        )
        .map_err(Error::internal)?;
        Ok((workspace, commit_rev(tx)?))
    }

    pub fn workspaces(&self) -> Result<Vec<Workspace>, Error> {
        select(&self.db, "SELECT * FROM workspaces ORDER BY name", [], workspace_row)
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
        Ok((workspace, commit_rev(tx)?))
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
        Ok((workspace, commit_rev(tx)?))
    }

    pub fn workspace_tasks(&self, id: &WorkspaceId) -> Result<Vec<TaskRow>, Error> {
        select(&self.db, "SELECT * FROM tasks WHERE workspace = ?1 ORDER BY rowid", [&id.0], task_row)
    }

    /// Deletes the workspace with its tasks and their terminal and browser rows.
    pub fn remove_workspace(&mut self, id: &WorkspaceId) -> Result<u64, Error> {
        let tx = self.db.transaction().map_err(Error::internal)?;
        let in_workspace = "task IN (SELECT id FROM tasks WHERE workspace = ?1)";
        for table in ["panels", "browsers"] {
            tx.execute(&format!("DELETE FROM {table} WHERE {in_workspace}"), [&id.0]).map_err(Error::internal)?;
        }
        tx.execute("DELETE FROM tasks WHERE workspace = ?1", [&id.0]).map_err(Error::internal)?;
        tx.execute("DELETE FROM workspaces WHERE id = ?1", [&id.0]).map_err(Error::internal)?;
        commit_rev(tx)
    }

    pub fn set_harness(&mut self, id: &str, harness: &Harness) -> Result<u64, Error> {
        let tx = self.db.transaction().map_err(Error::internal)?;
        let upsert = format!(
            "{INSERT_HARNESS} ON CONFLICT(id) DO UPDATE SET argv = excluded.argv, start = excluded.start,
             resume = excluded.resume, install = excluded.install"
        );
        tx.execute(&upsert, harness_values(id, harness)).map_err(Error::internal)?;
        commit_rev(tx)
    }

    pub fn harness(&self, id: &str) -> Result<Option<Harness>, Error> {
        let row: Option<([String; 3], String)> = self
            .db
            .query_row("SELECT argv, start, resume, install FROM harnesses WHERE id = ?1", [id], |row| {
                Ok(([row.get(0)?, row.get(1)?, row.get(2)?], row.get(3)?))
            })
            .optional()
            .map_err(Error::internal)?;
        row.map(|([argv, start, resume], install)| {
            let args = |json: &str| serde_json::from_str(json).map_err(Error::internal);
            Ok(Harness { argv: args(&argv)?, start: args(&start)?, resume: args(&resume)?, install })
        })
        .transpose()
    }

    pub fn create_task(&mut self, mut row: TaskRow) -> Result<TaskRow, Error> {
        let tx = self.db.transaction().map_err(Error::internal)?;
        row.rev = bump_rev(&tx)?;
        tx.execute(
            INSERT_TASK,
            params![
                row.id.0,
                row.workspace.0,
                row.parent.as_ref().map(|parent| &parent.0),
                row.title,
                to_wire(&row.state),
                row.folder,
                to_wire(&row.mode),
                row.directory,
                row.worktree.as_ref().map(|worktree| &worktree.path),
                row.worktree.as_ref().map(|worktree| &worktree.branch),
                row.harness,
                row.session.id.0,
                row.session.started,
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
        select(&self.db, "SELECT * FROM tasks ORDER BY rowid", [], task_row)
    }

    /// Whether a task, live or ended, holds the worktree at `path`.
    pub fn holds_worktree(&self, path: &str) -> Result<bool, Error> {
        self.db
            .query_row("SELECT EXISTS(SELECT 1 FROM tasks WHERE worktree = ?1)", [path], |row| row.get(0))
            .map_err(Error::internal)
    }

    pub fn worktrees_ended_by(&self, cutoff: i64) -> Result<Vec<(TaskId, Worktree)>, Error> {
        let sql = "SELECT * FROM tasks WHERE worktree IS NOT NULL AND ended_at IS NOT NULL AND ended_at <= ?1";
        let rows: Vec<TaskRow> = select(&self.db, sql, [cutoff], task_row)?;
        Ok(rows.into_iter().filter_map(|row| Some((row.id, row.worktree?))).collect())
    }

    /// Files the task and every task below it in `workspace`, each at a rev of its own, in tree order.
    pub fn refile(&mut self, task: &TaskId, workspace: &WorkspaceId) -> Result<Vec<TaskRow>, Error> {
        let tx = self.db.transaction().map_err(Error::internal)?;
        let tree: Vec<String> = select(
            &tx,
            "WITH RECURSIVE tree(id) AS (SELECT ?1 UNION SELECT tasks.id FROM tasks JOIN tree ON tasks.parent = tree.id)
             SELECT id FROM tree",
            [&task.0],
            |row| row.get(0),
        )?;
        let mut refiled = Vec::new();
        for id in tree {
            let rev = bump_rev(&tx)?;
            let row = tx
                .query_row(
                    "UPDATE tasks SET workspace = ?2, rev = ?3 WHERE id = ?1 RETURNING *",
                    params![id, workspace.0, rev.cast_signed()],
                    task_row,
                )
                .map_err(Error::internal)?;
            refiled.push(row);
        }
        tx.commit().map_err(Error::internal)?;
        Ok(refiled)
    }

    pub fn save_task(&mut self, row: &mut TaskRow) -> Result<u64, Error> {
        let tx = self.db.transaction().map_err(Error::internal)?;
        row.rev = bump_rev(&tx)?;
        tx.execute(
            "UPDATE tasks SET title = ?2, state = ?3, session = ?4, session_started = ?5, ended_at = ?6, archived_at = ?7,
             rev = ?8, parent = ?9
             WHERE id = ?1",
            params![
                row.id.0,
                row.title,
                to_wire(&row.state),
                row.session.id.0,
                row.session.started,
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
        revise_task(self.db.transaction().map_err(Error::internal)?, id)
    }

    /// Every task's browser panels, each task's in its order.
    pub fn browsers(&self) -> Result<HashMap<TaskId, Vec<BrowserPanel>>, Error> {
        let rows: Vec<(TaskId, BrowserPanel)> =
            select(&self.db, "SELECT task, panel, url, title, load FROM browsers ORDER BY place, rowid", [], |row| {
                let panel = BrowserPanel {
                    panel: BrowserId(row.get(1)?),
                    url: row.get(2)?,
                    title: row.get(3)?,
                    load: row.get::<_, i64>(4)?.cast_unsigned(),
                };
                Ok((TaskId(row.get(0)?), panel))
            })?;
        let mut browsers: HashMap<TaskId, Vec<BrowserPanel>> = HashMap::new();
        for (task, panel) in rows {
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
        let rows: Vec<PanelRow> = select(&tx, "SELECT id, task, name FROM panels ORDER BY rowid", [], |row| {
            Ok(PanelRow { id: PtyId(row.get(0)?), task: TaskId(row.get(1)?), name: row.get(2)? })
        })?;
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
    fn a_task_keeps_its_browser_panels_in_the_order_they_moved_to_and_a_new_one_joins_the_end() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("store.db");
        let mut store = Store::open(&path).expect("a fresh store");
        let (workspace, _) = store.add_workspace("w", "/w", Color::Auto).expect("a workspace");
        store
            .db
            .execute(
                "INSERT INTO tasks (id, workspace, title, state, folder, mode, directory, harness, session,
                 session_started, rev)
                 VALUES ('tsk_1', ?1, 't', 'pinned', '/w', 'in_place', '/w', 'h', 's', 0, 1)",
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
    fn a_fresh_store_runs_claude_code_until_the_user_sets_it_otherwise() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("store.db");
        let mut store = Store::open(&path).expect("a fresh store");
        let claude_code = Harness {
            argv: vec!["claude".into()],
            start: vec!["--session-id".into(), "{session}".into()],
            resume: vec!["--resume".into(), "{session}".into()],
            install: "npm install -g @anthropic-ai/claude-code".into(),
        };
        assert_eq!(store.harness("claude-code").expect("harness"), Some(claude_code.clone()));
        let set = Harness { argv: vec!["claude".into(), "--verbose".into()], ..claude_code };
        store.set_harness("claude-code", &set).expect("harness.set");
        drop(store);
        let store = Store::open(&path).expect("reopen");
        assert_eq!(store.harness("claude-code").expect("harness"), Some(set));
    }

    #[test]
    fn a_store_whose_harnesses_hold_hooks_opens_without_them() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("store.db");
        Connection::open(&path)
            .and_then(|db| {
                db.execute_batch(
                    "CREATE TABLE harnesses (id TEXT PRIMARY KEY, argv TEXT NOT NULL, start TEXT NOT NULL,
                         resume TEXT NOT NULL, install TEXT NOT NULL, hooks TEXT NOT NULL);
                     INSERT INTO harnesses VALUES ('aider', '[\"aider\"]', '[]', '[]', 'pip install aider', 'none');",
                )
            })
            .expect("a store of hooked harnesses");
        let mut store = Store::open(&path).expect("open");
        let aider =
            Harness { argv: vec!["aider".into()], start: vec![], resume: vec![], install: "pip install aider".into() };
        assert_eq!(store.harness("aider").expect("harness"), Some(aider.clone()));
        store.set_harness("aider", &aider).expect("harness.set");
    }
}
