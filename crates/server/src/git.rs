//! Running git in a repo or folder.

use std::path::Path;
use std::process::{Command, Output};

use octopus_proto::Error;

pub(crate) fn git_command(repo: &Path, args: &[&str]) -> Command {
    let mut command = Command::new("git");
    command.arg("-C").arg(repo).args(args);
    command
}

pub(crate) fn run_git(repo: &Path, args: &[&str]) -> Result<Output, Error> {
    git_command(repo, args).output().map_err(|error| Error::internal(format!("run git: {error}")))
}

/// Git's output, or a `conflict` with its stderr when it fails.
pub(crate) fn git_output(repo: &Path, args: &[&str]) -> Result<Vec<u8>, Error> {
    let output = run_git(repo, args)?;
    if output.status.success() {
        return Ok(output.stdout);
    }
    Err(Error::conflict(format!("git {}: {}", args.join(" "), String::from_utf8_lossy(&output.stderr).trim())))
}

pub(crate) fn git(repo: &Path, args: &[&str]) -> Result<(), Error> {
    git_output(repo, args).map(drop)
}
