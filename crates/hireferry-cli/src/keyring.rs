//! OS credential store adapters. Secret values never appear in command arguments.
use crate::error::{Error, Result};

pub fn unavailable() -> Error {
    Error::new(
        "keyring_unavailable",
        "The system credential store is unavailable. Use --storage file to select private file storage.",
    )
}

#[cfg(target_os = "linux")]
fn command(action: &str, key: &str, secret: Option<&[u8]>) -> Result<std::process::Output> {
    use std::{
        io::Write,
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    let mut command = Command::new("secret-tool");
    command.arg(action);
    if action == "store" {
        command.arg("--label=HireFerry CLI");
    }
    command
        .args(["service", "hireferry-cli", "connection", key])
        .stdin(if secret.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|_| unavailable())?;
    if let Some(secret) = secret {
        let result = child
            .stdin
            .take()
            .ok_or_else(unavailable)
            .and_then(|mut input| {
                input
                    .write_all(secret)
                    .and_then(|_| input.write_all(b"\n"))
                    .map_err(|_| unavailable())
            });
        if result.is_err() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(unavailable());
        }
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return child.wait_with_output().map_err(|_| unavailable()),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(25)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(unavailable());
            }
        }
    }
}

#[cfg(target_os = "linux")]
pub fn read(key: &str) -> Result<Option<Vec<u8>>> {
    let output = command("lookup", key, None)?;
    if output.status.success() {
        let mut secret = output.stdout;
        if secret.last() == Some(&b'\n') {
            secret.pop();
        }
        Ok(Some(secret))
    } else if output.status.code() == Some(1) && output.stderr.is_empty() {
        Ok(None)
    } else {
        Err(unavailable())
    }
}

#[cfg(target_os = "linux")]
pub fn write(key: &str, secret: &[u8]) -> Result<()> {
    if command("store", key, Some(secret))?.status.success() {
        Ok(())
    } else {
        Err(unavailable())
    }
}

#[cfg(target_os = "linux")]
pub fn remove(key: &str) -> Result<()> {
    let output = command("clear", key, None)?;
    if output.status.success() || (output.status.code() == Some(1) && output.stderr.is_empty()) {
        Ok(())
    } else {
        Err(unavailable())
    }
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::{ffi::c_void, ptr};

    #[repr(C)]
    struct FileTime {
        low: u32,
        high: u32,
    }
    #[repr(C)]
    struct Credential {
        flags: u32,
        kind: u32,
        target: *mut u16,
        comment: *mut u16,
        modified: FileTime,
        blob_size: u32,
        blob: *mut u8,
        persist: u32,
        attributes_count: u32,
        attributes: *mut c_void,
        alias: *mut u16,
        username: *mut u16,
    }
    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn CredReadW(
            target: *const u16,
            kind: u32,
            flags: u32,
            credential: *mut *mut Credential,
        ) -> i32;
        fn CredWriteW(credential: *const Credential, flags: u32) -> i32;
        fn CredDeleteW(target: *const u16, kind: u32, flags: u32) -> i32;
        fn CredFree(buffer: *mut c_void);
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetLastError() -> u32;
    }

    fn target(key: &str) -> Vec<u16> {
        format!("HireFerry/CLI/{key}")
            .encode_utf16()
            .chain([0])
            .collect()
    }

    pub fn read(key: &str) -> Result<Option<Vec<u8>>> {
        let name = target(key);
        let mut credential: *mut Credential = ptr::null_mut();
        let success = unsafe { CredReadW(name.as_ptr(), 1, 0, &mut credential) };
        if success == 0 {
            return if unsafe { GetLastError() } == 1168 {
                Ok(None)
            } else {
                Err(unavailable())
            };
        }
        if credential.is_null() {
            return Err(unavailable());
        }
        let value = unsafe {
            if (*credential).blob_size == 0 || (*credential).blob.is_null() {
                None
            } else {
                Some(
                    std::slice::from_raw_parts(
                        (*credential).blob,
                        (*credential).blob_size as usize,
                    )
                    .to_vec(),
                )
            }
        };
        unsafe { CredFree(credential.cast()) };
        value.map(Some).ok_or_else(unavailable)
    }

    pub fn write(key: &str, secret: &[u8]) -> Result<()> {
        if secret.len() > 2560 {
            return Err(Error::new(
                "keyring_capacity",
                "Credentials exceed the Windows credential size limit. Use --storage file.",
            ));
        }
        let mut name = target(key);
        let mut username: Vec<_> = "HireFerry CLI".encode_utf16().chain([0]).collect();
        let credential = Credential {
            flags: 0,
            kind: 1,
            target: name.as_mut_ptr(),
            comment: ptr::null_mut(),
            modified: FileTime { low: 0, high: 0 },
            blob_size: secret.len() as u32,
            blob: secret.as_ptr().cast_mut(),
            persist: 2,
            attributes_count: 0,
            attributes: ptr::null_mut(),
            alias: ptr::null_mut(),
            username: username.as_mut_ptr(),
        };
        if unsafe { CredWriteW(&credential, 0) } != 0 {
            Ok(())
        } else {
            Err(unavailable())
        }
    }

    pub fn remove(key: &str) -> Result<()> {
        let name = target(key);
        if unsafe { CredDeleteW(name.as_ptr(), 1, 0) } != 0 || unsafe { GetLastError() } == 1168 {
            Ok(())
        } else {
            Err(unavailable())
        }
    }
}

#[cfg(windows)]
pub use windows::{read, remove, write};

#[cfg(target_os = "macos")]
#[path = "keyring_macos.rs"]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::{read, remove, write};

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub fn read(_: &str) -> Result<Option<Vec<u8>>> {
    Err(unavailable())
}
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub fn write(_: &str, _: &[u8]) -> Result<()> {
    Err(unavailable())
}
#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub fn remove(_: &str) -> Result<()> {
    Err(unavailable())
}
