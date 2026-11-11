//! Persistent application diagnostics, independent of managed processes' output logs.
use std::backtrace::Backtrace;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::Path;
use std::sync::Mutex;

struct FileLogger(Mutex<Option<File>>);

static LOGGER: FileLogger = FileLogger(Mutex::new(None));

impl FileLogger {
    fn write(&self, message: &str, sync: bool) {
        let line = format!("{} {message}\n", chrono::Utc::now().to_rfc3339());
        let mut guard = self.0.lock().unwrap_or_else(|err| err.into_inner());
        if let Some(file) = guard.as_mut() {
            // File is unbuffered: each record reaches the OS before returning to the renderer.
            let _ = file.write_all(line.as_bytes());
            if sync {
                let _ = file.sync_data();
            }
        }
    }
}

impl log::Log for FileLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Warn
            || metadata.target().starts_with("simple_rust_process_manager")
            || (metadata.level() <= log::Level::Debug
                && metadata.target().starts_with("eframe::native"))
    }

    fn log(&self, record: &log::Record<'_>) {
        if self.enabled(record.metadata()) {
            self.write(
                &format!("{} {}: {}", record.level(), record.target(), record.args()),
                record.level() == log::Level::Error,
            );
        }
    }

    fn flush(&self) {
        if let Some(file) = self
            .0
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .as_mut()
        {
            let _ = file.sync_data();
        }
    }
}

pub fn init(directory: &Path) -> io::Result<()> {
    std::fs::create_dir_all(directory)?;
    let path = directory.join(format!(
        "process-manager-{}-{}.log",
        chrono::Utc::now().format("%Y%m%dT%H%M%S%.9fZ"),
        std::process::id()
    ));
    let file = OpenOptions::new().create_new(true).write(true).open(path)?;
    *LOGGER.0.lock().unwrap_or_else(|err| err.into_inner()) = Some(file);
    log::set_logger(&LOGGER).map_err(io::Error::other)?;
    log::set_max_level(log::LevelFilter::Debug);

    let previous_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Do not depend on RUST_BACKTRACE being set in an operator's environment.
        LOGGER.write(
            &format!(
                "PANIC thread={:?}: {info}\n{}",
                std::thread::current().name(),
                Backtrace::force_capture()
            ),
            true,
        );
        previous_hook(info);
    }));
    log::info!(
        "PM starting version={} pid={} os={} arch={}",
        env!("CARGO_PKG_VERSION"),
        std::process::id(),
        std::env::consts::OS,
        std::env::consts::ARCH
    );
    Ok(())
}

#[cfg(windows)]
pub fn observe_window(hwnd: windows_sys::Win32::Foundation::HWND) {
    use windows_sys::Win32::UI::{Shell::SetWindowSubclass, WindowsAndMessaging::GetClientRect};
    use windows_sys::Win32::{Foundation::RECT, UI::HiDpi::GetDpiForWindow};
    unsafe {
        if SetWindowSubclass(hwnd, Some(window_proc), 1, 0) == 0 {
            log::error!("Could not install native window diagnostics");
        }
        let mut rect: RECT = std::mem::zeroed();
        if GetClientRect(hwnd, &mut rect) != 0 {
            log::info!(
                "Window initial client_px={}x{} dpi={}",
                rect.right - rect.left,
                rect.bottom - rect.top,
                GetDpiForWindow(hwnd)
            );
        }
    }
}

// Observe sent window messages BEFORE winit can resize/reconfigure the GPU surface.
// A frame-level hook alone misses a resize that panics before the next app update.
#[cfg(windows)]
unsafe extern "system" fn window_proc(
    hwnd: windows_sys::Win32::Foundation::HWND,
    message: u32,
    wparam: usize,
    lparam: isize,
    subclass: usize,
    _data: usize,
) -> isize {
    use windows_sys::Win32::UI::{
        Shell::{DefSubclassProc, RemoveWindowSubclass},
        WindowsAndMessaging::*,
    };
    use windows_sys::Win32::{Foundation::RECT, UI::HiDpi::GetDpiForWindow};
    match message {
        WM_SIZE => log::info!(
            "WM_SIZE client_px={}x{} kind={} (0=restored 1=minimized 2=maximized) dpi={}",
            lparam as u32 & 0xffff,
            (lparam as u32 >> 16) & 0xffff,
            wparam,
            GetDpiForWindow(hwnd)
        ),
        WM_DPICHANGED => {
            let rect = &*(lparam as *const RECT);
            log::info!(
                "WM_DPICHANGED dpi={}x{} suggested_outer_px=({},{})-({},{})",
                wparam & 0xffff,
                (wparam >> 16) & 0xffff,
                rect.left,
                rect.top,
                rect.right,
                rect.bottom
            );
        }
        WM_WINDOWPOSCHANGED => {
            let pos = &*(lparam as *const WINDOWPOS);
            log::info!(
                "WM_WINDOWPOSCHANGED outer_px=({},{}) {}x{} flags={:#x} dpi={}",
                pos.x,
                pos.y,
                pos.cx,
                pos.cy,
                pos.flags,
                GetDpiForWindow(hwnd)
            );
        }
        WM_DISPLAYCHANGE => log::info!(
            "WM_DISPLAYCHANGE display_px={}x{} bits_per_pixel={}",
            lparam as u32 & 0xffff,
            (lparam as u32 >> 16) & 0xffff,
            wparam
        ),
        WM_CLOSE => log::info!("WM_CLOSE received"),
        WM_NCDESTROY => {
            log::info!("WM_NCDESTROY received");
            RemoveWindowSubclass(hwnd, Some(window_proc), subclass);
        }
        _ => {}
    }
    DefSubclassProc(hwnd, message, wparam, lparam)
}

#[cfg(test)]
mod tests {
    #[test]
    fn persists_panic_and_framework_errors() {
        const CHILD: &str = "PM_TEST_DIAGNOSTICS_DIRECTORY";
        if let Some(directory) = std::env::var_os(CHILD) {
            super::init(std::path::Path::new(&directory)).unwrap();
            log::warn!(target: "wgpu_core", "test GPU warning");
            log::error!(target: "eframe::native::run", "test event loop failure");
            panic!("test diagnostic panic");
        }

        let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(".tmp")
            .join(format!("diagnostics-test-{}", uuid::Uuid::new_v4()));
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "diagnostics::tests::persists_panic_and_framework_errors",
                "--nocapture",
            ])
            .env(CHILD, &directory)
            .env_remove("RUST_BACKTRACE")
            .output()
            .unwrap();
        assert!(!output.status.success());
        let path = std::fs::read_dir(&directory)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let text = std::fs::read_to_string(path).unwrap();
        assert!(text.contains("PM starting version="), "{text}");
        assert!(text.contains("test GPU warning"), "{text}");
        assert!(text.contains("test event loop failure"), "{text}");
        assert!(text.contains("PANIC thread="), "{text}");
        assert!(text.contains("test diagnostic panic"), "{text}");
        assert!(text.contains("diagnostics.rs:"), "{text}");
        assert!(
            text.contains("\n   0:"),
            "Expected a captured backtrace: {text}"
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
}
