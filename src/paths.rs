//! Filesystem locations for Sage (XDG-style).

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

pub fn data_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("SAGE_DATA_DIR") {
        return PathBuf::from(dir);
    }
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        return PathBuf::from(xdg).join("sage");
    }
    home_dir()
        .map(|h| h.join(".local/share/sage"))
        .unwrap_or_else(|| PathBuf::from("./.sage/data"))
}

pub fn config_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("SAGE_CONFIG_DIR") {
        return PathBuf::from(dir);
    }
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg).join("sage");
    }
    home_dir()
        .map(|h| h.join(".config/sage"))
        .unwrap_or_else(|| PathBuf::from("./.sage/config"))
}

pub fn config_file() -> PathBuf {
    config_dir().join("config.toml")
}

pub fn temp_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("SAGE_TEMP_DIR") {
        return PathBuf::from(dir);
    }
    let base = std::env::temp_dir();
    // SAFETY: `geteuid` takes no arguments and has no memory-safety preconditions.
    let uid = unsafe { libc::geteuid() };
    base.join(format!("sage-{uid}"))
}

pub fn recordings_dir() -> PathBuf {
    data_dir().join("recordings")
}

pub fn models_dir() -> PathBuf {
    data_dir().join("models")
}

pub fn run_dir() -> PathBuf {
    data_dir().join("run")
}

pub fn database_path() -> PathBuf {
    data_dir().join("sage.db")
}

pub fn legacy_history_path() -> PathBuf {
    data_dir().join("history.json")
}

pub fn daemon_pid_path() -> PathBuf {
    run_dir().join("daemon.pid")
}

pub fn daemon_socket_path() -> PathBuf {
    run_dir().join("sage.sock")
}

pub fn ensure_dirs() -> std::io::Result<()> {
    for directory in [
        config_dir(),
        data_dir(),
        recordings_dir(),
        models_dir(),
        run_dir(),
        temp_dir(),
    ] {
        fs::create_dir_all(&directory)?;
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_env;

    #[test]
    fn override_data_dir_via_env() {
        let _g = test_env::lock();
        let dir = std::env::temp_dir().join(format!("sage-path-data-{}", std::process::id()));
        std::env::set_var("SAGE_DATA_DIR", &dir);
        assert_eq!(data_dir(), dir);
        assert_eq!(database_path(), dir.join("sage.db"));
        std::env::remove_var("SAGE_DATA_DIR");
    }

    #[test]
    fn ensure_dirs_creates_layout() {
        let _g = test_env::lock();
        let data = std::env::temp_dir().join(format!("sage-path-ensure-{}", std::process::id()));
        let cfg = data.join("cfg");
        let tmp = data.join("tmp");
        let _ = fs::remove_dir_all(&data);
        std::env::set_var("SAGE_DATA_DIR", &data);
        std::env::set_var("SAGE_CONFIG_DIR", &cfg);
        std::env::set_var("SAGE_TEMP_DIR", &tmp);
        ensure_dirs().expect("ensure_dirs");
        assert!(recordings_dir().is_dir());
        assert!(run_dir().is_dir());
        assert!(models_dir().is_dir());
        assert!(config_dir().is_dir());
        assert!(temp_dir().is_dir());
        std::env::remove_var("SAGE_DATA_DIR");
        std::env::remove_var("SAGE_CONFIG_DIR");
        std::env::remove_var("SAGE_TEMP_DIR");
        let _ = fs::remove_dir_all(&data);
    }

    #[test]
    fn default_temp_dir_uses_process_uid_without_environment() {
        let _g = test_env::lock();
        let previous_temp = std::env::var_os("SAGE_TEMP_DIR");
        let previous_uid = std::env::var_os("UID");
        std::env::remove_var("SAGE_TEMP_DIR");
        std::env::remove_var("UID");
        // SAFETY: `geteuid` takes no arguments and has no memory-safety preconditions.
        let expected = std::env::temp_dir().join(format!("sage-{}", unsafe { libc::geteuid() }));
        assert_eq!(temp_dir(), expected);
        if let Some(value) = previous_temp {
            std::env::set_var("SAGE_TEMP_DIR", value);
        }
        if let Some(value) = previous_uid {
            std::env::set_var("UID", value);
        }
    }
}
