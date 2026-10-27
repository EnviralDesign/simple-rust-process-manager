use std::{io, path::Path, time::Duration};

use windows_sys::Win32::{
    Foundation::{CloseHandle, GetLastError, BOOL, ERROR_ALREADY_EXISTS, HANDLE, HWND, LPARAM},
    System::Threading::{CreateMutexW, ReleaseMutex},
    UI::WindowsAndMessaging::{
        EnumWindows, GetPropW, IsIconic, SetForegroundWindow, SetPropW, ShowWindowAsync, SW_RESTORE,
    },
};

pub struct SingleInstance {
    handle: HANDLE,
    name: Vec<u16>,
}

impl SingleInstance {
    pub fn acquire(config_path: &Path) -> io::Result<Option<Self>> {
        let name = mutex_name(config_path)?;
        let handle = unsafe { CreateMutexW(std::ptr::null(), 1, name.as_ptr()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            // The first process may still be creating its window.
            for _ in 0..100 {
                let mut search = WindowSearch {
                    name: &name,
                    hwnd: std::ptr::null_mut(),
                };
                unsafe {
                    EnumWindows(Some(find_window), &mut search as *mut _ as LPARAM);
                }
                if !search.hwnd.is_null() {
                    unsafe {
                        if IsIconic(search.hwnd) != 0 {
                            ShowWindowAsync(search.hwnd, SW_RESTORE);
                        }
                        SetForegroundWindow(search.hwnd);
                    }
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            unsafe {
                CloseHandle(handle);
            }
            return Ok(None);
        }
        Ok(Some(Self { handle, name }))
    }

    pub fn register_window(&self, hwnd: HWND) -> io::Result<()> {
        if unsafe { SetPropW(hwnd, self.name.as_ptr(), 1usize as HANDLE) } == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        unsafe {
            ReleaseMutex(self.handle);
            CloseHandle(self.handle);
        }
    }
}

fn mutex_name(config_path: &Path) -> io::Result<Vec<u16>> {
    let full_path = match config_path.canonicalize() {
        Ok(path) => path,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            let parent = config_path
                .parent()
                .ok_or_else(|| io::Error::other("Config has no parent directory"))?;
            parent.canonicalize()?.join(
                config_path
                    .file_name()
                    .ok_or_else(|| io::Error::other("Config has no filename"))?,
            )
        }
        Err(err) => return Err(err),
    };
    // Stable FNV-1a keeps even long configuration paths within the Windows name limit.
    let normalized = full_path
        .to_string_lossy()
        .replace('/', "\\")
        .to_lowercase();
    let hash = normalized
        .bytes()
        .fold(0xcbf29ce484222325u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
        });
    Ok(format!("Local\\SimpleRustProcessManager-{hash:016x}")
        .encode_utf16()
        .chain(Some(0))
        .collect())
}

struct WindowSearch<'a> {
    name: &'a [u16],
    hwnd: HWND,
}

unsafe extern "system" fn find_window(hwnd: HWND, param: LPARAM) -> BOOL {
    let search = &mut *(param as *mut WindowSearch<'_>);
    if !GetPropW(hwnd, search.name.as_ptr()).is_null() {
        search.hwnd = hwnd;
        return 0;
    }
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_path_identity_is_stable_and_scoped() {
        let directory = std::env::current_dir().unwrap();
        let path = directory.join("processes.json");
        assert_eq!(
            mutex_name(&path).unwrap(),
            mutex_name(&directory.join(".").join("processes.json")).unwrap()
        );
        assert_ne!(
            mutex_name(&path).unwrap(),
            mutex_name(&directory.join("other-processes.json")).unwrap()
        );
    }

    #[test]
    fn mutex_is_exclusive_and_released_on_drop() {
        let path = std::env::current_dir()
            .unwrap()
            .join(format!("instance-test-{}.json", std::process::id()));
        let first = SingleInstance::acquire(&path).unwrap().unwrap();
        assert!(SingleInstance::acquire(&path).unwrap().is_none());
        let other_path = path.with_extension("other.json");
        let other = SingleInstance::acquire(&other_path).unwrap().unwrap();
        drop(first);
        assert!(SingleInstance::acquire(&path).unwrap().is_some());
        drop(other);
    }
}
