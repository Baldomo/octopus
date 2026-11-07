use std::io;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

use crate::account::AccountId;
use crate::schema::{self, Fields};
use crate::types::{Stream, Task, TaskId, Workspace, WorkspaceId};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub id: u64,
    pub op: String,
    pub args: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Response {
    pub id: u64,
    #[serde(flatten)]
    pub outcome: Outcome,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Ok(Value),
    Err(Error),
}

impl<T: Serialize> From<Result<T, Error>> for Outcome {
    fn from(result: Result<T, Error>) -> Self {
        match result.and_then(|ok| serde_json::to_value(ok).map_err(Error::internal)) {
            Ok(value) => Self::Ok(value),
            Err(error) => Self::Err(error),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Push {
    pub rev: u64,
    #[serde(flatten)]
    pub event: Event,
}

/// `FileChanged` and `AccountUpdated` carry the server's current rev and move no state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "ev")]
pub enum Event {
    #[serde(rename = "task.updated")]
    TaskUpdated { task: Box<Task> },
    #[serde(rename = "workspace.updated")]
    WorkspaceUpdated { workspace: Workspace },
    /// Its tasks are gone with it.
    #[serde(rename = "workspace.removed")]
    WorkspaceRemoved { workspace: WorkspaceId },
    #[serde(rename = "harness.updated")]
    HarnessUpdated { harness: String },
    #[serde(rename = "file.changed")]
    FileChanged { task: TaskId, path: String },
    /// A merge added entries to the server's log of the account.
    #[serde(rename = "account.updated")]
    AccountUpdated { account: AccountId },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum ServerFrame {
    Response(Response),
    Push(Push),
}

/// The first line of a data stream. The server answers with one `Outcome` line, then the stream's bytes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StreamHeader {
    pub open: Stream,
    /// The client's name, as on its control connection.
    pub client: String,
}

/// What the open reply of a `Stream::Read` carries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FileSize {
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, thiserror::Error)]
#[serde(tag = "code", rename_all = "snake_case", deny_unknown_fields)]
pub enum Error {
    #[error("missing fields: {}", fields.join(", "))]
    MissingField { fields: Vec<String> },
    #[error("unknown fields: {}", fields.join(", "))]
    UnknownField { fields: Vec<String> },
    #[error("invalid args: {message}")]
    InvalidArgs { message: String },
    #[error("unknown op: {op}")]
    UnknownOp { op: String },
    #[error("not found: {what}")]
    NotFound { what: String },
    #[error("terminal locked by {holder}")]
    PtyLocked { holder: String },
    #[error("conflict: {message}")]
    Conflict { message: String },
    #[error("internal: {message}")]
    Internal { message: String },
    #[error("this client is not paired with the server; pair it with an invite first")]
    Unpaired,
    #[error("the pairing code is spent, expired, or another server's")]
    CodeRefused,
}

impl Error {
    pub fn internal(error: impl std::fmt::Display) -> Self {
        Self::Internal { message: error.to_string() }
    }

    pub fn invalid(error: impl std::fmt::Display) -> Self {
        Self::InvalidArgs { message: error.to_string() }
    }

    pub fn conflict(message: impl std::fmt::Display) -> Self {
        Self::Conflict { message: message.to_string() }
    }

    pub fn not_found(what: impl std::fmt::Display) -> Self {
        Self::NotFound { what: what.to_string() }
    }
}

pub trait Op: Fields + Serialize + DeserializeOwned {
    const NAME: &'static str;
    const DOC: &'static str;
    type Ok: Serialize + DeserializeOwned;
}

/// One newline-terminated JSON frame.
pub fn line(frame: &impl Serialize) -> String {
    let mut line = serde_json::to_string(frame).expect("protocol frames serialize");
    line.push('\n');
    line
}

/// Parses an op's args, reporting every absent field at every depth at once and rejecting any
/// undeclared one.
pub fn parse_args<T: Fields + DeserializeOwned>(args: &Value) -> Result<T, Error> {
    schema::check(T::fields(), args)?;
    serde_json::from_value(args.clone()).map_err(Error::invalid)
}

/// A 1-byte buffer leaves every byte after the reply line in the stream.
pub(crate) const REPLY_BUFFER: usize = 1;

pub(crate) fn decode_reply(reply: &[u8]) -> io::Result<Outcome> {
    serde_json::from_slice(reply).map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

/// Sends `header` as one line and reads the one `Outcome` line that answers it, leaving every later byte unread.
pub async fn open<S: AsyncRead + AsyncWrite + Unpin>(stream: &mut S, header: &impl Serialize) -> io::Result<Outcome> {
    stream.write_all(line(header).as_bytes()).await?;
    read_outcome(stream).await
}

/// Reads the one `Outcome` line next on `stream`, leaving every later byte unread.
pub async fn read_outcome<S: AsyncRead + Unpin>(stream: &mut S) -> io::Result<Outcome> {
    let mut reply = Vec::new();
    BufReader::with_capacity(REPLY_BUFFER, stream).read_until(b'\n', &mut reply).await?;
    decode_reply(&reply)
}

/// Parses one control request line, keeping its id when the line is JSON but not a request.
pub fn parse_request(line: &str) -> Result<Request, (u64, Error)> {
    request_from(serde_json::from_str(line).map_err(|error| (0, Error::invalid(error)))?)
}

/// A request from a line already parsed as JSON.
pub fn request_from(value: Value) -> Result<Request, (u64, Error)> {
    let id = value.get("id").and_then(Value::as_u64).unwrap_or(0);
    serde_json::from_value(value).map_err(|error| (id, Error::invalid(error)))
}
