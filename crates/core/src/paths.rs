use std::io;
use std::path::{Path, PathBuf};

/// Data directory. `APARK_HOME` overrides the platform default
/// (~/Library/Application Support/Apark, %APPDATA%\Apark, ~/.local/share/Apark).
pub fn home() -> PathBuf {
    if let Some(p) = std::env::var_os("APARK_HOME") {
        return PathBuf::from(p);
    }
    dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("Apark")
}

pub fn ensure_home() -> io::Result<PathBuf> {
    let h = home();
    std::fs::create_dir_all(&h)?;
    Ok(h)
}

/// Atomically write a file readable only by the current user.
pub fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&tmp, path)
}
