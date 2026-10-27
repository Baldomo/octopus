pub mod cache;
pub mod connection;
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
    fn a_server_on_a_loopback_endpoint_is_this_machine() {
        let key = DeviceKey::generate().public();
        let endpoints = [
            ("127.0.0.1:7300", false),
            ("[::1]:7300", false),
            ("localhost:7300", false),
            ("10.0.0.2:7300", true),
            ("mac-mini.tailnet.ts.net:7300", true),
        ];
        for (endpoint, another) in endpoints {
            assert_eq!(Address::Quic { endpoint: endpoint.into(), key }.is_another_machine(), another, "{endpoint}");
        }
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
        let me = Identity { name: "laptop".into(), key: DeviceKey::generate() };
        let connection = Connection::connect(&Address::Local { socket }, &me).await.expect("connect");
        tokio::time::timeout(Duration::from_secs(5), connection.closed()).await.expect("closed resolves");
        let call = connection.call_raw("workspace.list", serde_json::json!({}));
        assert!(tokio::time::timeout(Duration::from_secs(5), call).await.expect("no hang").is_err());
    }
}
