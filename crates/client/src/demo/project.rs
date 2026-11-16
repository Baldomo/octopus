//! The demo computer's one project, tidewatch: a tree of invented files held in memory, which every task shares.

use std::collections::{BTreeMap, BTreeSet};

use octopus_proto::Error;
use octopus_proto::ops::{Entry, EntryKind, Folder, Match};

/// Where the project sits on the demo computer, and the folder above it.
pub(super) const ROOT: &str = "/work/tidewatch";
const PARENT: &str = "/work";

macro_rules! files {
    ($($path:literal),* $(,)?) => {
        [$(($path, include_bytes!(concat!("project/", $path)).as_slice())),*]
    };
}

const FILES: [(&str, &[u8]); 12] = files!(
    "README.md",
    "Makefile",
    "data/harbor.csv",
    "docs/notes.md",
    "scripts/check.sh",
    "src/chart.js",
    "src/forecast.js",
    "src/index.html",
    "src/style.css",
    "src/tides.js",
    "tests/forecast.test.js",
    "tests/harness.js",
);

/// Each file by its path under the project, and each directory, the project itself as `""`.
pub(super) struct Project {
    files: BTreeMap<String, Vec<u8>>,
    dirs: BTreeSet<String>,
}

/// A path under the project, `.` for the project itself, with no `..` leading out of it.
fn relative(path: &str) -> Result<String, Error> {
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => return Err(Error::invalid(format!("{path} leads out of the task's directory"))),
            part => parts.push(part),
        }
    }
    if path.starts_with('/') {
        return Err(Error::invalid(format!("{path} is not relative to the task's directory")));
    }
    Ok(parts.join("/"))
}

fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(parent, _)| parent)
}

fn under(path: &str, dir: &str) -> bool {
    dir.is_empty() || path == dir || path.starts_with(&format!("{dir}/"))
}

impl Project {
    pub(super) fn new() -> Self {
        let mut project = Self { files: BTreeMap::new(), dirs: BTreeSet::from([String::new()]) };
        for (path, bytes) in FILES {
            project.put(path, bytes.to_vec());
        }
        project
    }

    fn put(&mut self, path: &str, bytes: Vec<u8>) {
        self.add_dirs(parent(path));
        self.files.insert(path.to_string(), bytes);
    }

    fn add_dirs(&mut self, dir: &str) {
        let mut at = dir;
        while !at.is_empty() {
            self.dirs.insert(at.to_string());
            at = parent(at);
        }
    }

    fn exists(&self, path: &str) -> bool {
        self.files.contains_key(path) || self.dirs.contains(path)
    }

    fn missing(path: &str) -> Error {
        Error::not_found(format!("{path} in the task's directory"))
    }

    pub(super) fn file(&self, path: &str) -> Result<&[u8], Error> {
        let path = relative(path)?;
        self.files.get(&path).map(Vec::as_slice).ok_or_else(|| Self::missing(&path))
    }

    pub(super) fn list(&self, path: &str) -> Result<Vec<Entry>, Error> {
        let dir = relative(path)?;
        if !self.dirs.contains(&dir) {
            return Err(Self::missing(&dir));
        }
        let named = |path: &String| -> Option<String> {
            (*path != dir && parent(path) == dir).then(|| path.rsplit('/').next().unwrap_or_default().to_string())
        };
        let dirs = self.dirs.iter().filter_map(named);
        let files = self.files.keys().filter_map(named);
        let mut entries: Vec<Entry> = dirs
            .map(|name| Entry { name, kind: EntryKind::Directory })
            .chain(files.map(|name| Entry { name, kind: EntryKind::File }))
            .collect();
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(entries)
    }

    /// Resolves to the path written.
    pub(super) fn write(&mut self, path: &str, bytes: Vec<u8>) -> Result<String, Error> {
        let path = relative(path)?;
        if path.is_empty() || self.dirs.contains(&path) {
            return Err(Error::conflict(format!("{path} is a directory")));
        }
        self.put(&path, bytes);
        Ok(path)
    }

    pub(super) fn create(&mut self, path: &str, kind: EntryKind) -> Result<String, Error> {
        let path = relative(path)?;
        if path.is_empty() || self.exists(&path) {
            return Err(Error::conflict(format!("{path} exists")));
        }
        match kind {
            EntryKind::File => self.put(&path, Vec::new()),
            EntryKind::Directory => self.add_dirs(&path),
        }
        Ok(path)
    }

    /// Copies `from` and everything under it to `to`, keeping `from` unless `moving`.
    pub(super) fn copy(&mut self, from: &str, to: &str, moving: bool) -> Result<String, Error> {
        let (from, to) = (relative(from)?, relative(to)?);
        if from.is_empty() || !self.exists(&from) {
            return Err(Self::missing(&from));
        }
        if to.is_empty() || self.exists(&to) || under(&to, &from) {
            return Err(Error::conflict(format!("{to} exists or sits inside {from}")));
        }
        let moved = |path: &String| format!("{to}{}", &path[from.len()..]);
        let files: Vec<(String, Vec<u8>)> = self
            .files
            .iter()
            .filter(|(path, _)| under(path, &from))
            .map(|(path, bytes)| (moved(path), bytes.clone()))
            .collect();
        let dirs: Vec<String> = self.dirs.iter().filter(|path| under(path, &from)).map(moved).collect();
        if moving {
            self.delete(&from)?;
        }
        dirs.iter().for_each(|dir| self.add_dirs(dir));
        files.into_iter().for_each(|(path, bytes)| self.put(&path, bytes));
        Ok(to)
    }

    pub(super) fn delete(&mut self, path: &str) -> Result<String, Error> {
        let path = relative(path)?;
        if path.is_empty() || !self.exists(&path) {
            return Err(Self::missing(&path));
        }
        self.files.retain(|file, _| !under(file, &path));
        self.dirs.retain(|dir| !under(dir, &path) || dir.is_empty());
        Ok(path)
    }

    pub(super) fn search(&self, pattern: &str) -> Vec<Match> {
        let mut matches = Vec::new();
        for (path, bytes) in &self.files {
            for (at, text) in String::from_utf8_lossy(bytes).lines().enumerate() {
                if text.contains(pattern) {
                    matches.push(Match { path: path.clone(), line: at as u64 + 1, text: text.to_string() });
                }
            }
        }
        matches
    }

    /// A folder of the demo computer, for a new task to pick: `/`, `/work`, or the project and its directories.
    pub(super) fn folder(&self, folder: &str) -> Result<Folder, Error> {
        let trimmed = folder.trim_end_matches('/');
        let unknown = || Error::not_found(format!("{folder} on the demo computer"));
        let (folders, repo) = match trimmed {
            "" => (vec!["work".to_string()], None),
            PARENT => (vec!["tidewatch".to_string()], None),
            _ => {
                let dir = trimmed.strip_prefix(ROOT).ok_or_else(unknown)?;
                let dir = dir.strip_prefix('/').unwrap_or(dir);
                let listed = self.list(if dir.is_empty() { "." } else { dir }).map_err(|_| unknown())?;
                let dirs = listed.into_iter().filter(|entry| entry.kind == EntryKind::Directory);
                (dirs.map(|entry| entry.name).collect(), Some(ROOT.to_string()))
            }
        };
        let branch = repo.as_ref().map(|_| "main".to_string());
        Ok(Folder { folder: if trimmed.is_empty() { "/".into() } else { trimmed.into() }, repo, branch, folders })
    }
}
