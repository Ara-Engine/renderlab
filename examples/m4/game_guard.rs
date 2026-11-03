/// Check game processes without launching a shell inside replay measurements.
#[cfg(windows)]
pub fn active() -> std::io::Result<bool> {
    use std::{ffi::c_void, mem::size_of};

    #[repr(C)]
    struct ProcessEntry {
        size: u32,
        usage: u32,
        pid: u32,
        heap: usize,
        module: u32,
        threads: u32,
        parent: u32,
        priority: i32,
        flags: u32,
        executable: [u16; 260],
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> *mut c_void;
        fn Process32FirstW(snapshot: *mut c_void, entry: *mut ProcessEntry) -> i32;
        fn Process32NextW(snapshot: *mut c_void, entry: *mut ProcessEntry) -> i32;
        fn CloseHandle(handle: *mut c_void) -> i32;
    }
    // The snapshot owns one kernel handle. Close it on both success and enumeration failure.
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(2, 0);
        if snapshot == -1isize as *mut c_void {
            return Err(std::io::Error::last_os_error());
        }
        let mut entry: ProcessEntry = std::mem::zeroed();
        entry.size = size_of::<ProcessEntry>() as u32;
        let mut found = Process32FirstW(snapshot, &mut entry);
        let result = loop {
            if found == 0 {
                let error = std::io::Error::last_os_error();
                break if error.raw_os_error() == Some(18) {
                    Ok(false)
                } else {
                    Err(error)
                };
            }
            let end = entry.executable.iter().position(|&c| c == 0).unwrap_or(260);
            let name = String::from_utf16_lossy(&entry.executable[..end]).to_ascii_lowercase();
            if name == "valorant-win64-shipping.exe"
                || name.starts_with("fortniteclient")
                || name == "eldenring.exe"
            {
                break Ok(true);
            }
            found = Process32NextW(snapshot, &mut entry);
        };
        CloseHandle(snapshot);
        result
    }
}

#[cfg(not(windows))]
pub fn active() -> std::io::Result<bool> {
    Ok(false)
}
