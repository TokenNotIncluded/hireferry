//! Native macOS Keychain access. Credential bytes never enter a process argument.
//! The C declarations match Apple's public Security.framework headers.
use crate::error::{Error, Result};
use std::{
    ffi::{c_char, c_void},
    ptr,
};

const SERVICE: &[u8] = b"hireferry-cli";
const NOT_FOUND: i32 = -25300;
const DUPLICATE: i32 = -25299;
const MAX_SECRET: u32 = 64 * 1024;

#[link(name = "Security", kind = "framework")]
unsafe extern "C" {
    fn SecKeychainSetUserInteractionAllowed(allowed: u8) -> i32;
    fn SecKeychainFindGenericPassword(
        keychains: *const c_void,
        service_len: u32,
        service: *const c_char,
        account_len: u32,
        account: *const c_char,
        password_len: *mut u32,
        password: *mut *mut c_void,
        item: *mut *const c_void,
    ) -> i32;
    fn SecKeychainAddGenericPassword(
        keychain: *const c_void,
        service_len: u32,
        service: *const c_char,
        account_len: u32,
        account: *const c_char,
        password_len: u32,
        password: *const c_void,
        item: *mut *const c_void,
    ) -> i32;
    fn SecKeychainItemModifyAttributesAndData(
        item: *const c_void,
        attributes: *const c_void,
        length: u32,
        data: *const c_void,
    ) -> i32;
    fn SecKeychainItemFreeContent(attributes: *mut c_void, data: *mut c_void) -> i32;
    fn SecKeychainItemDelete(item: *const c_void) -> i32;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRelease(value: *const c_void);
}

struct Item(*const c_void);
impl Drop for Item {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // The find function returns a retained reference, owned by this guard.
            unsafe { CFRelease(self.0) };
        }
    }
}

struct Password(*mut c_void);
impl Drop for Password {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // The find function allocates this buffer; free it with its paired API.
            unsafe { SecKeychainItemFreeContent(ptr::null_mut(), self.0) };
        }
    }
}

fn prepare(key: &str) -> Result<()> {
    if key.len() != 64 || !key.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(Error::storage());
    }
    // Applies only to this process. A locked/unavailable keychain must fail
    // promptly instead of opening an unbounded desktop prompt in a script.
    if unsafe { SecKeychainSetUserInteractionAllowed(0) } != 0 {
        return Err(crate::keyring::unavailable());
    }
    Ok(())
}

fn find(key: &str, read_data: bool) -> Result<Option<(Item, Password, u32)>> {
    prepare(key)?;
    let mut item = ptr::null();
    let mut data = ptr::null_mut();
    let mut length = 0;
    // All input buffers live throughout the call. Optional output pointers are
    // null when only the item reference is needed for update or deletion.
    let status = unsafe {
        SecKeychainFindGenericPassword(
            ptr::null(),
            SERVICE.len() as u32,
            SERVICE.as_ptr().cast(),
            key.len() as u32,
            key.as_ptr().cast(),
            if read_data {
                &mut length
            } else {
                ptr::null_mut()
            },
            if read_data {
                &mut data
            } else {
                ptr::null_mut()
            },
            &mut item,
        )
    };
    let item = Item(item);
    let data = Password(data);
    match status {
        0 if !item.0.is_null() => Ok(Some((item, data, length))),
        NOT_FOUND => Ok(None),
        _ => Err(crate::keyring::unavailable()),
    }
}

pub fn read(key: &str) -> Result<Option<Vec<u8>>> {
    let Some((_item, password, length)) = find(key, true)? else {
        return Ok(None);
    };
    if length == 0 || length > MAX_SECRET || password.0.is_null() {
        return Err(Error::storage());
    }
    // Security.framework owns at least `length` bytes until the guard is dropped.
    let bytes = unsafe { std::slice::from_raw_parts(password.0.cast::<u8>(), length as usize) };
    Ok(Some(bytes.to_vec()))
}

pub fn write(key: &str, secret: &[u8]) -> Result<()> {
    prepare(key)?;
    if secret.is_empty() || secret.len() > MAX_SECRET as usize {
        return Err(Error::storage());
    }
    let update = |item: Item| {
        // The item is retained for this call and input bytes remain borrowed.
        let status = unsafe {
            SecKeychainItemModifyAttributesAndData(
                item.0,
                ptr::null(),
                secret.len() as u32,
                secret.as_ptr().cast(),
            )
        };
        if status == 0 {
            Ok(())
        } else {
            Err(crate::keyring::unavailable())
        }
    };
    if let Some((item, _, _)) = find(key, false)? {
        return update(item);
    }
    // No item reference is requested, so no CF object needs releasing here.
    let status = unsafe {
        SecKeychainAddGenericPassword(
            ptr::null(),
            SERVICE.len() as u32,
            SERVICE.as_ptr().cast(),
            key.len() as u32,
            key.as_ptr().cast(),
            secret.len() as u32,
            secret.as_ptr().cast(),
            ptr::null_mut(),
        )
    };
    if status == 0 {
        return Ok(());
    }
    // Another process may have created the same account after our first lookup.
    if status == DUPLICATE {
        if let Some((item, _, _)) = find(key, false)? {
            return update(item);
        }
    }
    Err(crate::keyring::unavailable())
}

pub fn remove(key: &str) -> Result<()> {
    let Some((item, _, _)) = find(key, false)? else {
        return Ok(());
    };
    // Deletion does not release the retained reference; Item handles that.
    match unsafe { SecKeychainItemDelete(item.0) } {
        0 | NOT_FOUND => Ok(()),
        _ => Err(crate::keyring::unavailable()),
    }
}
