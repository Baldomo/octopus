//! The environment the user's own terminal starts in, which every agent the server starts runs in.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use octopus_proto::Error;

/// The PATH an agent's program is found on, the login environment's own.
pub fn search_path(environment: &[(String, String)]) -> Result<OsString, Error> {
    let path = environment.iter().find(|(key, _)| key == "PATH").map(|(_, path)| OsString::from(path));
    path.ok_or_else(|| Error::internal("the login environment sets no PATH"))
}

/// The account's login shell, which a shell panel runs.
pub fn shell() -> Result<PathBuf, Error> {
    use nix::unistd::{User, getuid};

    let uid = getuid();
    let account = User::from_uid(uid).map_err(Error::internal)?;
    Ok(account.ok_or_else(|| Error::internal(format!("uid {uid} has no account")))?.shell)
}

/// Runs the account's login shell interactively, as a terminal window does, in `directory`, so whatever the
/// profile sets up per directory applies. The marker sets apart whatever the profile prints itself, and `exec`
/// skips the logout files, which could print after. The output goes to files rather than pipes, so a daemon the
/// profile leaves running cannot hold the read open.
pub fn resolve(directory: &Path) -> Result<Vec<(String, String)>, Error> {
    use std::io::{Read, Seek};
    use std::process::{Command, Stdio};

    const MARKER: &str = "octopus-login-environment\n";
    let shell = shell()?;
    let failed =
        |error: &dyn std::fmt::Display| Error::internal(format!("{} as a login shell: {error}", shell.display()));
    let run = || -> std::io::Result<_> {
        let (mut stdout, mut stderr) = (tempfile::tempfile()?, tempfile::tempfile()?);
        let status = Command::new(&shell)
            .args(["-l", "-i", "-c", &format!("printf '{MARKER}'; exec env -0")])
            .current_dir(directory)
            .stdin(Stdio::null())
            .stdout(stdout.try_clone()?)
            .stderr(stderr.try_clone()?)
            .status()?;
        let read = |file: &mut std::fs::File| -> std::io::Result<String> {
            let mut bytes = Vec::new();
            file.rewind()?;
            file.read_to_end(&mut bytes)?;
            Ok(String::from_utf8_lossy(&bytes).into_owned())
        };
        Ok((status, read(&mut stdout)?, read(&mut stderr)?))
    };
    let (status, stdout, stderr) = run().map_err(|error| failed(&error))?;
    if !status.success() {
        return Err(failed(&format!("{status}: {}", stderr.trim())));
    }
    let (_, variables) = stdout.split_once(MARKER).ok_or_else(|| failed(&"printed no environment"))?;
    Ok(variables.split('\0').filter_map(|pair| pair.split_once('=')).map(|(k, v)| (k.into(), v.into())).collect())
}
