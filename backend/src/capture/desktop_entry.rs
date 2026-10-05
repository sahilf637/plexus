//! Linux desktop entry (`<app id>.desktop`). Desktop portals identify apps by it: without one,
//! GNOME refuses to bind a global shortcut for an app that isn't sandboxed. Installed packages
//! ship the file; for dev runs and portable builds Plexus writes its own copy to
//! `~/.local/share/applications`.

use std::fs;
use std::path::PathBuf;

use crate::error::Result;

/// Marks entries Plexus wrote itself, so they can be refreshed (e.g. when the binary moves)
/// without ever touching one installed by a package or by the user.
const MARKER: &str = "X-Plexus-Generated=true";

pub fn ensure(app_id: &str) -> Result<()> {
    let file_name = format!("{app_id}.desktop");
    let user_dir = data_home().join("applications");
    let ours = user_dir.join(&file_name);

    let installed_elsewhere = data_dirs().iter().any(|dir| dir.join("applications").join(&file_name).exists());
    let existing = fs::read_to_string(&ours).ok();
    if installed_elsewhere || existing.as_deref().is_some_and(|text| !text.contains(MARKER)) {
        return Ok(());
    }

    let entry = contents(&std::env::current_exe()?.to_string_lossy());
    if existing.as_deref() != Some(entry.as_str()) {
        fs::create_dir_all(&user_dir)?;
        fs::write(&ours, entry)?;
        tracing::info!(path = %ours.display(), "wrote desktop entry");
    }
    Ok(())
}

fn contents(exe: &str) -> String {
    // Debug builds point at a dev binary: keep them out of the app grid.
    let no_display = if cfg!(debug_assertions) { "true" } else { "false" };
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Plexus\n\
         Comment=Copy anything. Your second brain wires itself.\n\
         Exec=\"{}\"\n\
         Terminal=false\n\
         Categories=Office;Utility;\n\
         NoDisplay={no_display}\n\
         {MARKER}\n",
        exe.replace('\\', "\\\\").replace('"', "\\\"")
    )
}

fn data_home() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| std::env::home_dir().unwrap_or_default().join(".local/share"))
}

/// System-wide data directories, where packages install their desktop entries.
fn data_dirs() -> Vec<PathBuf> {
    let dirs = std::env::var("XDG_DATA_DIRS").ok().filter(|d| !d.is_empty());
    dirs.as_deref()
        .unwrap_or("/usr/local/share:/usr/share")
        .split(':')
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn quotes_the_binary_path() {
        let entry = super::contents("/opt/My Apps/plexus");
        assert!(entry.contains("Exec=\"/opt/My Apps/plexus\"\n"));
        assert!(entry.contains(super::MARKER));
    }
}
