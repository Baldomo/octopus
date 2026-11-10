pub mod account;
pub mod cache;
pub mod connection;
pub mod qr;
pub mod servers;
pub mod tunnel;

pub use connection::{CallError, Connection, DataStream, Identity};

/// Ends its task when dropped, so background work ends with what owns it.
struct Task(tokio::task::JoinHandle<()>);

impl Drop for Task {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use octopus_proto::device::DeviceKey;
    use octopus_proto::local;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    use crate::account::Book;
    use crate::servers::Address;
    use crate::{Connection, Identity};

    fn address(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("octopus-client-{}-{name}.sock", std::process::id()))
    }

    #[test]
    fn a_list_from_before_accounts_gives_its_paired_servers_to_the_account() {
        use crate::servers::{Config, Servers};
        let dir = std::env::temp_dir().join(format!("octopus-client-{}-migrate", std::process::id()));
        let _fresh = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("a config dir");
        let desk = DeviceKey::generate().public();
        let listed = serde_json::json!({"servers": [
            {"name": "laptop", "address": {"path": "local", "socket": "/tmp/octopus.sock"}},
            {"name": "desk", "address": {"path": "remote", "key": desk, "relay": "https://relay.example./", "addresses": []}},
        ]});
        std::fs::write(dir.join("servers.json"), listed.to_string()).expect("write the list");
        let config = Config::at(dir.clone());
        let me = Identity::open("laptop".into(), DeviceKey::generate(), &config).expect("open");
        let servers = Servers::load(&config).expect("load");
        assert_eq!(servers.servers.len(), 1, "the list keeps the server on this machine alone");
        let reachable: Vec<String> = servers.reachable(&me).into_iter().map(|server| server.name).collect();
        assert_eq!(reachable, ["laptop", "desk"]);
        let again = Identity::open("laptop".into(), DeviceKey::generate(), &config).expect("open again");
        assert_eq!(again.account.state().servers.len(), 1, "the account kept in the file lists the server once");
        std::fs::remove_dir_all(&dir).expect("clean up");
    }

    #[tokio::test]
    async fn a_call_after_the_server_hangs_up_fails_instead_of_waiting() {
        let socket = address("closed");
        let mut listener = local::Listener::bind(&socket).expect("bind");
        let hosted =
            serde_json::json!({"key": DeviceKey::generate().public(), "relay": "https://relay.example./", "name": "s"});
        tokio::spawn(async move {
            let (read, mut write) = tokio::io::split(listener.accept().await.expect("accept"));
            let mut lines = BufReader::new(read).lines();
            lines.next_line().await.expect("hello");
            write.write_all(b"{\"id\":1,\"ok\":{\"server\":\"s\",\"rev\":0}}\n").await.expect("welcome");
            // The account syncs, then syncs again once it lists this server as this client's.
            for id in [2, 3] {
                let sync: serde_json::Value =
                    serde_json::from_str(&lines.next_line().await.expect("sync").expect("a line")).expect("a request");
                let synced = serde_json::json!({"id": id, "ok": {"account": sync["args"]["account"], "settings": sync["args"]["settings"], "server": hosted}});
                write.write_all(format!("{synced}\n").as_bytes()).await.expect("synced");
            }
        });
        let key = DeviceKey::generate();
        let me = Identity::new("laptop".into(), DeviceKey::from_pkcs8(key.pkcs8()).expect("a key"), Book::memory(&key))
            .expect("me");
        let connection = Connection::connect(&Address::Local { socket }, &me).await.expect("connect");
        tokio::time::timeout(Duration::from_secs(5), connection.closed()).await.expect("closed resolves");
        let call = connection.call_raw("workspace.list", serde_json::json!({}));
        assert!(tokio::time::timeout(Duration::from_secs(5), call).await.expect("no hang").is_err());
    }
}
