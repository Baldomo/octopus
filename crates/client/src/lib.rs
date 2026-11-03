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

    use crate::servers::Address;
    use crate::{Connection, Identity};

    fn address(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("octopus-client-{}-{name}.sock", std::process::id()))
    }

    #[test]
    fn a_remote_entry_sheds_its_addresses_and_keeps_its_key_and_relay() {
        use crate::servers::{Servers, strip_addresses};
        let desk = DeviceKey::generate().public();
        let relay = "https://relay.example./";
        let mut listed = serde_json::json!({"servers": [
            {"name": "laptop", "address": {"path": "local", "socket": "/tmp/octopus.sock"}},
            {"name": "desk", "address": {"path": "remote", "key": desk, "relay": relay, "addresses": ["127.0.0.1:7300"]}},
        ]});
        assert!(strip_addresses(&mut listed));
        let servers: Servers = serde_json::from_value(listed.clone()).expect("the stripped list parses");
        assert_eq!(servers.servers[1].address, Address::Remote { key: desk, relay: relay.parse().expect("a relay") });
        assert!(!strip_addresses(&mut listed), "a stripped list stays as it is");
    }

    #[test]
    fn a_listed_server_renames_to_a_free_name_and_forgets() {
        use crate::servers::{Server, Servers};
        let at = |name: &str| Server { name: name.into(), address: Address::Local { socket: address(name) } };
        let mut servers = Servers { servers: vec![at("laptop"), at("mac-mini")] };
        assert!(servers.rename("laptop", "mac-mini").is_err());
        assert!(servers.rename("laptop", "a/b").is_err());
        servers.rename("laptop", "desk").expect("rename");
        servers.forget("mac-mini").expect("forget");
        assert!(servers.forget("mac-mini").is_err());
        assert_eq!(servers.servers, [Server { name: "desk".into(), address: at("laptop").address }]);
    }

    #[tokio::test]
    async fn a_call_after_the_server_hangs_up_fails_instead_of_waiting() {
        let socket = address("closed");
        let mut listener = local::Listener::bind(&socket).expect("bind");
        tokio::spawn(async move {
            let (read, mut write) = tokio::io::split(listener.accept().await.expect("accept"));
            let mut hello = String::new();
            BufReader::new(read).read_line(&mut hello).await.expect("hello");
            write.write_all(b"{\"id\":1,\"ok\":{\"server\":\"s\",\"rev\":0}}\n").await.expect("welcome");
        });
        let me = Identity::new("laptop".into(), DeviceKey::generate());
        let connection = Connection::connect(&Address::Local { socket }, &me).await.expect("connect");
        tokio::time::timeout(Duration::from_secs(5), connection.closed()).await.expect("closed resolves");
        let call = connection.call_raw("workspace.list", serde_json::json!({}));
        assert!(tokio::time::timeout(Duration::from_secs(5), call).await.expect("no hang").is_err());
    }
}
