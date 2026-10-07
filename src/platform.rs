//! Platform integration shared by configuration, process control and diagnostics.
use std::path::{Path, PathBuf};

pub fn data_directory() -> PathBuf {
    let executable = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("."));
    data_directory_for(&executable)
}

fn data_directory_for(executable: &Path) -> PathBuf {
    #[cfg(target_os = "macos")]
    if let Some(bundle) = app_bundle(executable) {
        if is_installed_app(bundle, dirs::home_dir().as_deref()) {
            if let Some(directory) = dirs::data_local_dir() {
                return directory.join("Simple Rust Process Manager");
            }
        }
        return bundle
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf();
    }
    executable
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf()
}

#[cfg(target_os = "macos")]
fn app_bundle(executable: &Path) -> Option<&Path> {
    executable
        .parent()
        .filter(|p| p.file_name().is_some_and(|n| n == "MacOS"))
        .and_then(Path::parent)
        .filter(|p| p.file_name().is_some_and(|n| n == "Contents"))
        .and_then(Path::parent)
        .filter(|p| p.extension().is_some_and(|e| e == "app"))
}

#[cfg(target_os = "macos")]
fn is_installed_app(bundle: &Path, home: Option<&Path>) -> bool {
    bundle.starts_with("/Applications")
        || bundle.starts_with("/System/Applications")
        || bundle.starts_with("/System/Volumes/Data/Applications")
        || home.is_some_and(|home| bundle.starts_with(home.join("Applications")))
}

pub fn initialize() {
    #[cfg(target_os = "macos")]
    {
        use std::time::Duration;
        let mut paths = Vec::new();
        let shell = std::env::var_os("SHELL").unwrap_or_else(|| "/bin/zsh".into());
        if let Some(value) = login_shell_path(&shell, Duration::from_secs(3)) {
            paths.extend(std::env::split_paths(&value));
        }
        if let Some(value) = std::env::var_os("PATH") {
            paths.extend(std::env::split_paths(&value));
        }
        paths.extend([
            PathBuf::from("/opt/homebrew/bin"),
            PathBuf::from("/usr/local/bin"),
        ]);
        if let Some(home) = dirs::home_dir() {
            paths.extend([home.join(".local/bin"), home.join(".cargo/bin")]);
        }
        let mut seen = std::collections::HashSet::new();
        paths.retain(|path| !path.as_os_str().is_empty() && seen.insert(path.clone()));
        if let Ok(path) = std::env::join_paths(paths) {
            // Called by main before the runtime or worker threads start.
            std::env::set_var("PATH", path);
        }
        let _ = std::fs::create_dir_all(data_directory());
    }
}

#[cfg(target_os = "macos")]
fn login_shell_path(
    shell: &std::ffi::OsStr,
    timeout: std::time::Duration,
) -> Option<std::ffi::OsString> {
    use std::io::{Read, Seek, SeekFrom};
    use std::os::unix::{ffi::OsStrExt, process::CommandExt};
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};
    // A file keeps shell startup output bounded when we read it and avoids waiting
    // forever for stdout pipes inherited by background jobs in shell startup files.
    let mut output = tempfile::tempfile().ok()?;
    let mut child = Command::new(shell)
        .args(["-ilc", "printf '\\0%s\\0' \"$PATH\""])
        .stdin(Stdio::null())
        .stdout(output.try_clone().ok()?)
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .ok()?;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) => return None,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(25)),
            _ => {
                let _ = kill_process_group(child.id());
                let _ = child.wait();
                return None;
            }
        }
    }
    output.seek(SeekFrom::Start(0)).ok()?;
    let mut bytes = Vec::new();
    output.take(64 * 1024).read_to_end(&mut bytes).ok()?;
    let mut segments = bytes.split(|b| *b == 0);
    segments.next()?;
    let path = segments.next()?;
    segments.next()?; // Require the closing delimiter.
    Some(std::ffi::OsStr::from_bytes(path).to_os_string())
}

#[cfg(unix)]
fn signal_process_group(pid: u32, signal: i32) -> std::io::Result<()> {
    let pid = i32::try_from(pid)
        .ok()
        .filter(|pid| *pid > 1)
        .ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "Invalid process group")
        })?;
    // Each managed command is spawned with process_group(0), so its PID is its PGID.
    if unsafe { libc::kill(-pid, signal) } == 0 {
        Ok(())
    } else {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            Ok(())
        } else {
            Err(error)
        }
    }
}

#[cfg(unix)]
pub fn kill_process_group(pid: u32) -> std::io::Result<()> {
    signal_process_group(pid, libc::SIGKILL)
}

#[cfg(unix)]
pub fn stop_process_group(child: &mut std::process::Child) -> std::io::Result<()> {
    let pid = child.id();
    signal_process_group(pid, libc::SIGTERM)?;
    // Allow applications a short grace period, then kill stubborn descendants even
    // if their parent already exited. Never signal the manager's own process group.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
    while std::time::Instant::now() < deadline {
        let _ = child.try_wait(); // Reap the root so a zombie does not extend the grace period.
        if unsafe { libc::kill(-(pid as i32), 0) } != 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::ESRCH) {
                return Ok(());
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    kill_process_group(pid)
}

#[cfg(unix)]
fn process_system() -> &'static std::sync::Mutex<sysinfo::System> {
    static SYSTEM: std::sync::OnceLock<std::sync::Mutex<sysinfo::System>> =
        std::sync::OnceLock::new();
    SYSTEM.get_or_init(|| std::sync::Mutex::new(sysinfo::System::new()))
}

#[cfg(unix)]
pub fn refresh_process_resources() {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate};
    process_system()
        .lock()
        .unwrap()
        .refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing().with_cpu().with_memory(),
        );
}

#[cfg(unix)]
pub fn process_group_resources(root_pid: u32) -> Option<(u64, u64)> {
    let system = process_system().lock().unwrap();
    let mut cpu = 0_u64;
    let mut memory = 0_u64;
    let mut found = false;
    for (pid, process) in system.processes() {
        // Include grandchildren and reparented children that remain in our group.
        if unsafe { libc::getpgid(pid.as_u32() as i32) } == root_pid as i32 {
            found = true;
            cpu = cpu.saturating_add(process.accumulated_cpu_time().saturating_mul(10_000));
            memory = memory.saturating_add(process.memory());
        }
    }
    found.then_some((cpu, memory))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portable_data_stays_beside_binary() {
        assert_eq!(
            data_directory_for(Path::new("/tmp/manager/bin/manager")),
            PathBuf::from("/tmp/manager/bin")
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn portable_bundles_keep_each_stacks_data_beside_the_app() {
        for directory in [
            "/tmp/stacks/A",
            "/tmp/stacks/B",
            "/tmp/Applications",
            "/Applications Backup",
        ] {
            let executable =
                Path::new(directory).join("Renamed Manager.app/Contents/MacOS/manager");
            assert_eq!(data_directory_for(&executable), PathBuf::from(directory));
        }
        assert!(app_bundle(Path::new("/tmp/Contents/MacOS/manager")).is_none());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn installed_bundles_use_application_support() {
        let mut directories = vec![
            PathBuf::from("/Applications"),
            PathBuf::from("/Applications/Tools"),
            PathBuf::from("/System/Applications"),
            PathBuf::from("/System/Volumes/Data/Applications"),
        ];
        directories.push(dirs::home_dir().unwrap().join("Applications"));
        let expected = dirs::data_local_dir()
            .unwrap()
            .join("Simple Rust Process Manager");
        for directory in directories {
            let executable = directory.join("Process Manager.app/Contents/MacOS/manager");
            assert_eq!(data_directory_for(&executable), expected);
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn shell_path_ignores_startup_noise_and_times_out() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let shell = directory.path().join("shell");
        std::fs::write(
            &shell,
            "#!/bin/sh\necho startup-noise\nprintf '\\0/usr/local/bin:/usr/bin\\0'\n",
        )
        .unwrap();
        std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            login_shell_path(shell.as_os_str(), std::time::Duration::from_secs(1)),
            Some(std::ffi::OsString::from("/usr/local/bin:/usr/bin"))
        );
        std::fs::write(&shell, "#!/bin/sh\nsleep 60\n").unwrap();
        let start = std::time::Instant::now();
        assert!(
            login_shell_path(shell.as_os_str(), std::time::Duration::from_millis(50)).is_none()
        );
        assert!(start.elapsed() < std::time::Duration::from_secs(2));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_unsafe_process_groups() {
        assert!(kill_process_group(0).is_err());
        assert!(kill_process_group(1).is_err());
        assert!(kill_process_group(u32::MAX).is_err());
    }
}
