use std::path::PathBuf;

use anyhow::{Context, anyhow};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Server {
    pub name: String,
    pub socket: PathBuf,
}

impl Server {
    /// A client keys a server's tasks as `<name>/<task>`, and a relative socket would depend on where it started.
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.name.is_empty() || self.name.contains('/') {
            anyhow::bail!("a server name is non-empty and has no '/': {:?}", self.name);
        }
        if !self.socket.is_absolute() {
            anyhow::bail!("the socket of {} must be an absolute path: {}", self.name, self.socket.display());
        }
        Ok(())
    }
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
            Ok(text) => {
                let servers: Self = serde_json::from_str(&text).with_context(|| format!("parse {}", path.display()))?;
                servers
                    .servers
                    .iter()
                    .try_for_each(Server::validate)
                    .with_context(|| format!("check {}", path.display()))?;
                Ok(servers)
            }
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

    pub fn add(&mut self, server: Server) -> anyhow::Result<()> {
        server.validate()?;
        self.servers.retain(|known| known.name != server.name);
        self.servers.push(server);
        Ok(())
    }

    pub fn get(&self, name: &str) -> anyhow::Result<&Server> {
        self.servers
            .iter()
            .find(|server| server.name == name)
            .ok_or_else(|| anyhow!("no server named {name}; add it with `octopus server add`"))
    }
}
