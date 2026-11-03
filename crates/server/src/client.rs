//! Who a request or stream comes from, which holds and shows terminals.

use std::fmt;

use octopus_proto::Error;
use octopus_proto::device::PublicKey;

const NAME_CHARS: usize = 64;

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

/// A paired client's name is one line of at most 64 characters, so it reads whole wherever a client is listed.
pub fn check_name(name: &str) -> Result<(), Error> {
    if name.is_empty() || name.chars().count() > NAME_CHARS || name.chars().any(char::is_control) {
        return Err(Error::invalid(format!(
            "a client name is 1 to {NAME_CHARS} characters with no control characters, not {name:?}"
        )));
    }
    Ok(())
}
