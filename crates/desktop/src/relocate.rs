//! Moving the Mac app into Applications when it runs from a disk image or from the read-only copy macOS translocates
//! a downloaded app to, where the server it installs would not stay.

use std::path::Path;
use std::process::Command;

use anyhow::{Context, bail};
use objc2::MainThreadMarker;
use objc2_app_kit::{NSAlert, NSAlertFirstButtonReturn};
use objc2_foundation::{NSFileManager, NSString, NSURL};

const APPLICATIONS: &str = "/Applications";

/// Offers to move `bundle`, and on yes copies it into Applications, trashing an older copy there, opens the copy and
/// ends this process. Declined, the app runs where it is, and the next launch offers again.
pub fn offer(main_thread: MainThreadMarker, bundle: &Path) -> anyhow::Result<()> {
    let alert = NSAlert::new(main_thread);
    alert.setMessageText(&NSString::from_str("Move octopus to Applications?"));
    alert.setInformativeText(&NSString::from_str(
        "octopus is running from a disk image or a download, so the server it installs would stop when that goes. \
         From Applications it keeps running, wherever you later move it.",
    ));
    alert.addButtonWithTitle(&NSString::from_str("Move to Applications"));
    alert.addButtonWithTitle(&NSString::from_str("Not Now"));
    if alert.runModal() != NSAlertFirstButtonReturn {
        return Ok(());
    }
    let moved = Path::new(APPLICATIONS).join(bundle.file_name().expect("a bundle has a name"));
    move_to(bundle, &moved)?;
    Command::new("/usr/bin/open").arg("-n").arg(&moved).spawn().with_context(|| format!("open {}", moved.display()))?;
    std::process::exit(0)
}

/// The quarantine flag would send the copy back through translocation, so it goes; the app is notarized.
fn move_to(bundle: &Path, moved: &Path) -> anyhow::Result<()> {
    if moved.exists() {
        let url = NSURL::fileURLWithPath(&NSString::from_str(&moved.display().to_string()));
        NSFileManager::defaultManager()
            .trashItemAtURL_resultingItemURL_error(&url, None)
            .map_err(|error| anyhow::anyhow!("move {} to the Trash: {error}", moved.display()))?;
    }
    for (program, args) in [
        ("/usr/bin/ditto", vec![bundle.as_os_str(), moved.as_os_str()]),
        ("/usr/bin/xattr", vec!["-dr".as_ref(), "com.apple.quarantine".as_ref(), moved.as_os_str()]),
    ] {
        let status = Command::new(program).args(&args).status().with_context(|| format!("run {program}"))?;
        if !status.success() {
            bail!("{program} could not put octopus in {APPLICATIONS}: {status}");
        }
    }
    Ok(())
}
