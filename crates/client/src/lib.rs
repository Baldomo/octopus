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
