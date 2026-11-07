//! Who a request or stream comes from, which holds and shows terminals.

use std::fmt;

use octopus_proto::Error;
use octopus_proto::account::{self, NAME_CHARS};
use octopus_proto::device::PublicKey;

use crate::{Server, lock, now, report};

/// A paired device is the key it proved, whatever name it goes by; a client on the local socket, which only the
/// server's own OS user reaches, is the name it gives.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Client {
    Local(String),
    Paired(PublicKey),
}

impl fmt::Display for Client {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::Local(name) => f.write_str(name),
            Self::Paired(key) => write!(f, "{key}"),
        }
    }
}

pub(crate) fn check_name(name: &str) -> Result<(), Error> {
    if !account::is_name(name) {
        return Err(Error::invalid(format!(
            "a client name is 1 to {NAME_CHARS} characters with no control characters, not {name:?}"
        )));
    }
    Ok(())
}

impl Server {
    /// Only a paired client is seen: it alone is pinned.
    pub(crate) fn saw(&self, client: &Client) {
        let Client::Paired(key) = client else { return };
        if let Err(error) = lock(&self.state).store.client_seen(key, now()) {
            report!("record when {client} was seen: {error}");
        }
    }

    pub(crate) fn client_connected(&self, client: &Client) {
        self.saw(client);
        *lock(&self.clients).entry(client.clone()).or_default() += 1;
    }
}
