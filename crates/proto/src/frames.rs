use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::types::{Task, TaskId, Workspace};

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

impl From<Outcome> for Result<Value, Error> {
    fn from(outcome: Outcome) -> Self {
        match outcome {
            Outcome::Ok(value) => Ok(value),
            Outcome::Err(error) => Err(error),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Push {
    pub rev: u64,
    #[serde(flatten)]
    pub event: Event,
}

/// `FileChanged` carries the server's current rev and moves no state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "ev")]
pub enum Event {
    #[serde(rename = "task.updated")]
    TaskUpdated { task: Box<Task> },
    #[serde(rename = "workspace.added")]
    WorkspaceAdded { workspace: Workspace },
    #[serde(rename = "harness.updated")]
    HarnessUpdated { harness: String },
    #[serde(rename = "file.changed")]
    FileChanged { task: TaskId, path: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum ServerFrame {
    Response(Response),
    Push(Push),
}

/// The first line of a connection that carries one data stream. The server answers with one
/// `Outcome` line, then the stream's bytes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StreamHeader {
    pub open: String,
    pub client: String,
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

pub trait Op: Serialize + DeserializeOwned {
    const NAME: &'static str;
    const FIELDS: &'static [&'static str];
    type Ok: Serialize + DeserializeOwned;
}

/// One newline-terminated JSON frame.
pub fn line(frame: &impl Serialize) -> String {
    let mut line = serde_json::to_string(frame).expect("protocol frames serialize");
    line.push('\n');
    line
}

/// Parses an op's args, reporting every absent field at once and rejecting any undeclared one.
pub fn parse_args<O: Op>(args: &Value) -> Result<O, Error> {
    let Some(object) = args.as_object() else {
        return Err(Error::invalid("args must be an object"));
    };
    let missing: Vec<String> =
        O::FIELDS.iter().filter(|field| !object.contains_key(**field)).map(|field| field.to_string()).collect();
    if !missing.is_empty() {
        return Err(Error::MissingField { fields: missing });
    }
    let unknown: Vec<String> = object.keys().filter(|key| !O::FIELDS.contains(&key.as_str())).cloned().collect();
    if !unknown.is_empty() {
        return Err(Error::UnknownField { fields: unknown });
    }
    serde_json::from_value(args.clone()).map_err(Error::invalid)
}

/// Parses one control request line, keeping its id when the line is JSON but not a request.
pub fn parse_request(line: &str) -> Result<Request, (u64, Error)> {
    let value: Value = serde_json::from_str(line).map_err(|error| (0, Error::invalid(error)))?;
    let id = value.get("id").and_then(Value::as_u64).unwrap_or(0);
    serde_json::from_value(value).map_err(|error| (id, Error::invalid(error)))
}
