use std::path::PathBuf;

use anyhow::{Context, anyhow};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Server {
    pub name: String,
    pub socket: PathBuf,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Servers {
    pub servers: Vec<Server>,
}

fn path() -> anyhow::Result<PathBuf> {
    Ok(dirs::config_dir()
        .ok_or_else(|| anyhow!("no config directory on this platform"))?
        .join("octopus")
        .join("servers.json"))
}

impl Servers {
    pub fn load() -> anyhow::Result<Self> {
        let path = path()?;
        match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).with_context(|| format!("parse {}", path.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error).with_context(|| format!("read {}", path.display())),
        }
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, serde_json::to_string_pretty(self)?).with_context(|| format!("write {}", path.display()))
    }

    pub fn add(&mut self, server: Server) {
        self.servers.retain(|known| known.name != server.name);
        self.servers.push(server);
    }

    pub fn get(&self, name: &str) -> anyhow::Result<&Server> {
        self.servers
            .iter()
            .find(|server| server.name == name)
            .ok_or_else(|| anyhow!("no server named {name}; add it with `octopus server add`"))
    }
}
