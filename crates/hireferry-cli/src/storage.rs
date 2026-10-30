use crate::{
    error::{Error, Result},
    keyring,
    settings::StorageChoice,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const FILE_LIMIT: u64 = 64 * 1024;
const MAX_ACCOUNTS: usize = 64;

/// Immutable account identifier verified through this issuer's userinfo endpoint.
#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum AccountSubject {
    User(String),
    Administrator,
}

// Do not implement Debug: this type contains access and refresh credentials.
#[derive(Deserialize, Serialize)]
pub struct Credential {
    pub server: String,
    pub client_id: String,
    // None exists only in memory between token exchange and the initial userinfo
    // validation. The store refuses to persist an unbound credential.
    pub account: Option<AccountSubject>,
    pub scope: String,
    pub access_token: String,
    pub expires_at: i64,
    pub refresh_token: String,
    pub refresh_expires_at: i64,
    #[serde(default)]
    pub refresh_pending: bool,
}

pub struct Store {
    directory: PathBuf,
    path: PathBuf,
    marker: PathBuf,
    origin: String,
    client_id: String,
    account_name: String,
    lock_key: String,
    choice: StorageChoice,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Backend {
    File,
    Keyring,
}
#[derive(Serialize)]
pub struct StorageInfo {
    pub backend: &'static str,
    pub location: String,
}

#[derive(Deserialize, Serialize, Default)]
#[serde(deny_unknown_fields)]
struct AccountIndexData {
    #[serde(default)]
    default: Option<String>,
    #[serde(default)]
    accounts: BTreeMap<String, AccountIndexEntry>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct AccountIndexEntry {
    scope: String,
}

#[derive(Serialize)]
pub struct AccountList {
    pub default: Option<String>,
    pub accounts: Vec<AccountInfo>,
}

#[derive(Serialize)]
pub struct AccountInfo {
    pub name: String,
    pub scope: String,
    pub is_default: bool,
}

pub struct AccountRegistry {
    path: PathBuf,
}

impl Store {
    pub(crate) fn directory(&self) -> &Path {
        &self.directory
    }

    pub fn new(directory: PathBuf, origin: &str, client_id: &str, choice: StorageChoice) -> Self {
        Self::for_account(directory, origin, client_id, "default", choice)
            .expect("the built-in default account name is valid")
    }

    pub fn for_account(
        directory: PathBuf,
        origin: &str,
        client_id: &str,
        account_name: &str,
        choice: StorageChoice,
    ) -> Result<Self> {
        if !valid_account_name(account_name) {
            return Err(Error::new(
                "invalid_account_name",
                "Account names must be 1–32 lowercase letters, numbers, hyphens or underscores, and start with a letter or number.",
            ));
        }
        let lock_key = format!(
            "{:x}",
            Sha256::digest(format!("{origin}\0{client_id}").as_bytes())
        );
        let key = if account_name == "default" {
            lock_key.clone()
        } else {
            format!(
                "{:x}",
                Sha256::digest(format!("{lock_key}\0{account_name}").as_bytes())
            )
        };
        Ok(Self {
            path: directory.join(format!("{key}.credential")),
            marker: directory.join(format!("{key}.storage")),
            directory,
            origin: origin.to_owned(),
            client_id: client_id.to_owned(),
            account_name: account_name.to_owned(),
            lock_key,
            choice,
        })
    }

    pub fn account_name(&self) -> &str {
        &self.account_name
    }

    pub fn load(&self) -> Result<Option<Credential>> {
        if !self.check_directory(false)? {
            return Ok(None);
        }
        let bytes = match self.backend()? {
            Backend::Keyring => keyring::read(&self.keyring_key()?)?,
            Backend::File => private_read(&self.path)?
                .map(|bytes| unprotect(&bytes))
                .transpose()?,
        };
        bytes.map(|bytes| self.decode(&bytes)).transpose()
    }

    fn decode(&self, bytes: &[u8]) -> Result<Credential> {
        if bytes.len() as u64 > FILE_LIMIT {
            return Err(Error::storage());
        }
        let c: Credential = serde_json::from_slice(bytes).map_err(|_| Error::storage())?;
        if c.server != self.origin
            || c.client_id != self.client_id
            || match &c.account {
                Some(AccountSubject::User(id)) => {
                    id.is_empty() || id.len() > 256 || c.scope == "identity admin"
                }
                Some(AccountSubject::Administrator) => c.scope == "identity mcp",
                None => true,
            }
            || c.access_token.is_empty()
            || c.access_token.len() > 4096
            || c.refresh_token.is_empty()
            || c.refresh_token.len() > 4096
            || !matches!(
                c.scope.as_str(),
                "identity" | "identity mcp" | "identity admin"
            )
            || c.expires_at <= 0
            || c.refresh_expires_at <= 0
        {
            return Err(Error::storage());
        }
        Ok(c)
    }

    pub fn save(&self, credential: &Credential) -> Result<()> {
        if credential.server != self.origin || credential.client_id != self.client_id {
            return Err(Error::storage());
        }
        self.check_directory(true)?;
        let backend = self.backend()?;
        let bytes = serde_json::to_vec(credential).map_err(|_| Error::storage())?;
        self.decode(&bytes)?;
        match backend {
            Backend::Keyring => keyring::write(&self.keyring_key()?, &bytes)?,
            Backend::File => {
                if self.choice == StorageChoice::Auto && !self.marker.exists() {
                    eprintln!(
                        "System credential storage is unavailable. Using a private credential file: {}",
                        self.path.display()
                    );
                }
                private_write(&self.directory, &self.path, &protect(&bytes)?)?;
            }
        }
        private_write(
            &self.directory,
            &self.marker,
            if backend == Backend::File {
                b"file"
            } else {
                b"keyring"
            },
        )
    }

    pub fn remove(&self) -> Result<()> {
        if !self.check_directory(false)? {
            return Ok(());
        }
        match self.backend()? {
            Backend::Keyring => keyring::remove(&self.keyring_key()?)?,
            Backend::File => private_remove(&self.path)?,
        }
        private_remove(&self.marker)
    }

    fn backend(&self) -> Result<Backend> {
        let existing = match private_read(&self.marker)? {
            Some(bytes) => Some(match bytes.as_slice() {
                b"file" => Backend::File,
                b"keyring" => Backend::Keyring,
                _ => return Err(Error::storage()),
            }),
            None => fs::symlink_metadata(&self.path).ok().map(|_| Backend::File),
        };
        if let Some(existing) = existing {
            if matches!(
                (self.choice, existing),
                (StorageChoice::Keyring, Backend::File) | (StorageChoice::File, Backend::Keyring)
            ) {
                return Err(Error::new(
                    "storage_conflict",
                    "This connection uses a different credential store. Sign out with its current storage setting before changing it.",
                ));
            }
            // Never fall back after selecting a keyring: it may hold a newer refresh token.
            return Ok(existing);
        }
        match self.choice {
            StorageChoice::File => Ok(Backend::File),
            StorageChoice::Keyring => {
                keyring::read(&self.keyring_key()?)?;
                Ok(Backend::Keyring)
            }
            StorageChoice::Auto => match keyring::read(&self.keyring_key()?) {
                Ok(_) => Ok(Backend::Keyring),
                Err(e) if e.code == "keyring_unavailable" => Ok(Backend::File),
                Err(e) => Err(e),
            },
        }
    }

    pub fn info(&self) -> Result<StorageInfo> {
        Ok(match self.backend()? {
            Backend::File => StorageInfo {
                backend: "file",
                location: self.path.display().to_string(),
            },
            Backend::Keyring => StorageInfo {
                backend: "keyring",
                location: format!("hireferry-cli/{}", self.keyring_key()?),
            },
        })
    }

    fn keyring_key(&self) -> Result<String> {
        let directory = self
            .directory
            .canonicalize()
            .map_err(|_| Error::storage())?;
        let mut hash = Sha256::new();
        hash.update(self.origin.as_bytes());
        hash.update(b"\0");
        hash.update(self.client_id.as_bytes());
        hash.update(b"\0");
        hash.update(directory.as_os_str().as_encoded_bytes());
        if self.account_name != "default" {
            hash.update(b"\0");
            hash.update(self.account_name.as_bytes());
        }
        Ok(format!("{:x}", hash.finalize()))
    }

    pub async fn lock(&self) -> Result<StoreLock> {
        self.check_directory(true)?;
        let path = self.directory.join(format!("{}.lock", self.lock_key));
        let mut options = OpenOptions::new();
        options.create(true).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        if let Ok(metadata) = fs::symlink_metadata(&path) {
            check_file(&metadata)?;
        }
        let file = options.open(path).map_err(|_| Error::storage())?;
        check_file(&file.metadata().map_err(|_| Error::storage())?)?;
        tokio::task::spawn_blocking(move || StoreLock::acquire(file))
            .await
            .map_err(|_| Error::storage())?
    }

    fn check_directory(&self, create: bool) -> Result<bool> {
        let metadata = match fs::symlink_metadata(&self.directory) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && !create => return Ok(false),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let mut builder = fs::DirBuilder::new();
                builder.recursive(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::DirBuilderExt;
                    builder.mode(0o700);
                }
                builder
                    .create(&self.directory)
                    .map_err(|_| Error::storage())?;
                fs::symlink_metadata(&self.directory).map_err(|_| Error::storage())?
            }
            Err(_) => return Err(Error::storage()),
        };
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(insecure_store());
        }
        check_owner_mode(&metadata, 0o700)?;
        Ok(true)
    }
}

impl AccountRegistry {
    pub fn new(directory: &Path, origin: &str, client_id: &str) -> Self {
        let key = format!(
            "{:x}",
            Sha256::digest(format!("{origin}\0{client_id}").as_bytes())
        );
        Self {
            path: directory.join(format!("{key}.accounts")),
        }
    }

    pub fn default_account(&self) -> Result<Option<String>> {
        Ok(self.load()?.default)
    }

    pub fn list(&self) -> Result<AccountList> {
        let data = self.load()?;
        let default = data.default;
        let accounts = data
            .accounts
            .into_iter()
            .map(|(name, entry)| AccountInfo {
                is_default: default.as_deref() == Some(name.as_str()),
                name,
                scope: entry.scope,
            })
            .collect();
        Ok(AccountList { default, accounts })
    }

    pub fn contains(&self, name: &str) -> Result<bool> {
        Ok(self.load()?.accounts.contains_key(name))
    }

    pub fn record(&self, name: &str, scope: &str) -> Result<()> {
        if !valid_account_name(name) || !valid_scope(scope) {
            return Err(Error::storage());
        }
        let mut data = self.load()?;
        if data
            .accounts
            .get(name)
            .is_some_and(|entry| entry.scope == scope)
            && data.default.is_some()
        {
            return Ok(());
        }
        data.accounts.insert(
            name.to_owned(),
            AccountIndexEntry {
                scope: scope.to_owned(),
            },
        );
        if data.default.is_none() {
            data.default = Some(name.to_owned());
        }
        self.save(data)
    }

    pub fn set_default(&self, name: &str) -> Result<()> {
        if !valid_account_name(name) {
            return Err(Error::new(
                "invalid_account_name",
                "Account names must be 1–32 lowercase letters, numbers, hyphens or underscores, and start with a letter or number.",
            ));
        }
        let mut data = self.load()?;
        if !data.accounts.contains_key(name) {
            return Err(Error::new(
                "account_not_found",
                "No saved HireFerry account has that name. Run hfy auth accounts to list saved accounts.",
            ));
        }
        data.default = Some(name.to_owned());
        self.save(data)
    }

    pub fn remove(&self, name: &str) -> Result<Option<String>> {
        let mut data = self.load()?;
        data.accounts.remove(name);
        if data.default.as_deref() == Some(name) {
            data.default = data.accounts.keys().next().cloned();
        }
        let default = data.default.clone();
        self.save(data)?;
        Ok(default)
    }

    fn load(&self) -> Result<AccountIndexData> {
        let Some(bytes) = private_read(&self.path)? else {
            return Ok(AccountIndexData::default());
        };
        let data: AccountIndexData =
            serde_json::from_slice(&bytes).map_err(|_| Error::storage())?;
        validate_account_index(&data)?;
        Ok(data)
    }

    fn save(&self, data: AccountIndexData) -> Result<()> {
        validate_account_index(&data)?;
        let bytes = serde_json::to_vec(&data).map_err(|_| Error::storage())?;
        private_write(
            self.path.parent().ok_or_else(Error::storage)?,
            &self.path,
            &bytes,
        )
    }
}

fn validate_account_index(data: &AccountIndexData) -> Result<()> {
    if data.accounts.len() > MAX_ACCOUNTS
        || data
            .accounts
            .iter()
            .any(|(name, entry)| !valid_account_name(name) || !valid_scope(&entry.scope))
        || data
            .default
            .as_ref()
            .is_some_and(|name| !data.accounts.contains_key(name))
    {
        return Err(Error::storage());
    }
    Ok(())
}

fn valid_account_name(name: &str) -> bool {
    (1..=32).contains(&name.len())
        && name
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_'))
}

fn valid_scope(scope: &str) -> bool {
    matches!(scope, "identity" | "identity mcp" | "identity admin")
}

fn private_read(path: &Path) -> Result<Option<Vec<u8>>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(Error::storage()),
    };
    check_file(&metadata)?;
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    let file = options.open(path).map_err(|_| Error::storage())?;
    check_file(&file.metadata().map_err(|_| Error::storage())?)?;
    let mut bytes = Vec::new();
    file.take(FILE_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Error::storage())?;
    if bytes.len() as u64 > FILE_LIMIT {
        return Err(Error::storage());
    }
    Ok(Some(bytes))
}
fn private_write(directory: &Path, path: &Path, bytes: &[u8]) -> Result<()> {
    if let Ok(metadata) = fs::symlink_metadata(path) {
        check_file(&metadata)?;
    }
    let mut temporary = tempfile::Builder::new()
        .prefix(".hfy-")
        .tempfile_in(directory)
        .map_err(|_| Error::storage())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        temporary
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|_| Error::storage())?;
    }
    temporary.write_all(bytes).map_err(|_| Error::storage())?;
    temporary.flush().map_err(|_| Error::storage())?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|_| Error::storage())?;
    temporary.persist(path).map_err(|_| Error::storage())?;
    #[cfg(unix)]
    File::open(directory)
        .and_then(|file| file.sync_all())
        .map_err(|_| Error::storage())?;
    Ok(())
}
fn private_remove(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            check_file(&metadata)?;
            fs::remove_file(path).map_err(|_| Error::storage())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(Error::storage()),
    }
}
pub fn default_directory() -> Result<PathBuf> {
    #[cfg(windows)]
    let root = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    #[cfg(not(windows))]
    let root = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")));
    root.filter(|p| p.is_absolute())
        .map(|p| p.join("hireferry"))
        .ok_or_else(|| {
            Error::new(
                "config_directory",
                "Set --config-dir to a private directory for CLI credentials.",
            )
        })
}
fn check_file(metadata: &fs::Metadata) -> Result<()> {
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > FILE_LIMIT {
        return Err(insecure_store());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.nlink() != 1 {
            return Err(insecure_store());
        }
    }
    check_owner_mode(metadata, 0o600)
}
fn check_owner_mode(metadata: &fs::Metadata, allowed_mode: u32) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // geteuid has no pointer parameters and does not mutate process state.
        if metadata.uid() != unsafe { libc::geteuid() } || metadata.mode() & 0o777 != allowed_mode {
            return Err(insecure_store());
        }
    }
    #[cfg(not(unix))]
    let _ = (metadata, allowed_mode);
    Ok(())
}
fn insecure_store() -> Error {
    Error::new(
        "insecure_credential_store",
        "The credential store must be private, owned by you and free of symbolic links. On Unix, use mode 700 for the directory and 600 for files.",
    )
}

/// The OS releases this lock on process exit, including interrupted refreshes.
/// Keep the lock file: unlinking it could split waiters across different inodes.
pub struct StoreLock {
    file: File,
}
impl StoreLock {
    fn acquire(file: File) -> Result<Self> {
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            #[cfg(unix)]
            let locked = {
                use std::os::fd::AsRawFd;
                unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) == 0 }
            };
            #[cfg(windows)]
            let locked = {
                use std::os::windows::io::AsRawHandle;
                unsafe { file_lock_windows::LockFile(file.as_raw_handle(), 0, 0, 1, 0) != 0 }
            };
            #[cfg(not(any(unix, windows)))]
            let locked = false;
            if locked {
                return Ok(Self { file });
            }
            if Instant::now() >= deadline {
                return Err(Error::new(
                    "credentials_busy",
                    "Another hfy process is updating this connection. Try again when it finishes.",
                ));
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }
}
impl Drop for StoreLock {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            unsafe {
                libc::flock(self.file.as_raw_fd(), libc::LOCK_UN);
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            unsafe {
                file_lock_windows::UnlockFile(self.file.as_raw_handle(), 0, 0, 1, 0);
            }
        }
    }
}
#[cfg(windows)]
mod file_lock_windows {
    use std::ffi::c_void;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        pub fn LockFile(
            file: *mut c_void,
            low: u32,
            high: u32,
            length_low: u32,
            length_high: u32,
        ) -> i32;
        pub fn UnlockFile(
            file: *mut c_void,
            low: u32,
            high: u32,
            length_low: u32,
            length_high: u32,
        ) -> i32;
    }
}

#[cfg(unix)]
fn protect(bytes: &[u8]) -> Result<Vec<u8>> {
    Ok(bytes.to_vec())
}
#[cfg(unix)]
fn unprotect(bytes: &[u8]) -> Result<Vec<u8>> {
    Ok(bytes.to_vec())
}

#[cfg(windows)]
fn protect(bytes: &[u8]) -> Result<Vec<u8>> {
    windows::transform(bytes, true)
}
#[cfg(windows)]
fn unprotect(bytes: &[u8]) -> Result<Vec<u8>> {
    windows::transform(bytes, false)
}

#[cfg(not(any(unix, windows)))]
fn protect(_: &[u8]) -> Result<Vec<u8>> {
    Err(Error::new(
        "unsupported_platform",
        "Secure credential storage is not available on this platform.",
    ))
}
#[cfg(not(any(unix, windows)))]
fn unprotect(bytes: &[u8]) -> Result<Vec<u8>> {
    protect(bytes)
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::{ffi::c_void, ptr};

    #[repr(C)]
    struct Blob {
        length: u32,
        data: *mut u8,
    }

    #[link(name = "crypt32")]
    unsafe extern "system" {
        fn CryptProtectData(
            data: *const Blob,
            description: *const u16,
            entropy: *const Blob,
            reserved: *mut c_void,
            prompt: *const c_void,
            flags: u32,
            output: *mut Blob,
        ) -> i32;
        fn CryptUnprotectData(
            data: *const Blob,
            description: *mut *mut u16,
            entropy: *const Blob,
            reserved: *mut c_void,
            prompt: *const c_void,
            flags: u32,
            output: *mut Blob,
        ) -> i32;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn LocalFree(memory: *mut c_void) -> *mut c_void;
    }

    pub fn transform(bytes: &[u8], encrypt: bool) -> Result<Vec<u8>> {
        let input = Blob {
            length: u32::try_from(bytes.len()).map_err(|_| Error::storage())?,
            data: bytes.as_ptr().cast_mut(),
        };
        let mut output = Blob {
            length: 0,
            data: ptr::null_mut(),
        };
        // DPAPI binds encrypted bytes to the current Windows account. The
        // UI_FORBIDDEN flag prevents hidden prompts in unattended invocations.
        let success = unsafe {
            if encrypt {
                CryptProtectData(
                    &input,
                    ptr::null(),
                    ptr::null(),
                    ptr::null_mut(),
                    ptr::null(),
                    1,
                    &mut output,
                )
            } else {
                CryptUnprotectData(
                    &input,
                    ptr::null_mut(),
                    ptr::null(),
                    ptr::null_mut(),
                    ptr::null(),
                    1,
                    &mut output,
                )
            }
        };
        if success == 0 || output.data.is_null() {
            return Err(Error::storage());
        }
        // Both Windows functions return a LocalAlloc buffer owned by this call.
        let result =
            unsafe { std::slice::from_raw_parts(output.data, output.length as usize).to_vec() };
        unsafe { LocalFree(output.data.cast()) };
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn credential(server: &str) -> Credential {
        Credential {
            server: server.into(),
            client_id: "hireferry-cli".into(),
            account: Some(AccountSubject::User("member".into())),
            scope: "identity".into(),
            access_token: "test-access-secret".into(),
            expires_at: i64::MAX,
            refresh_token: "test-refresh-secret".into(),
            refresh_expires_at: i64::MAX,
            refresh_pending: false,
        }
    }
    fn store(directory: PathBuf, origin: &str) -> Store {
        Store::new(directory, origin, "hireferry-cli", StorageChoice::File)
    }
    #[test]
    fn server_and_client_credentials_are_separate() {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("credentials");
        let first = store(directory.clone(), "https://one.example");
        let second = store(directory.clone(), "https://two.example");
        let other = Store::new(
            directory,
            "https://one.example",
            "other-cli",
            StorageChoice::File,
        );
        assert!(first.load().unwrap().is_none());
        first.save(&credential("https://one.example")).unwrap();
        assert!(second.load().unwrap().is_none());
        assert!(other.load().unwrap().is_none());
        assert_eq!(
            first.load().unwrap().unwrap().access_token,
            "test-access-secret"
        );
        first.save(&credential("https://one.example")).unwrap();
        assert_eq!(fs::read_dir(&first.directory).unwrap().count(), 2);
        first.remove().unwrap();
        first.remove().unwrap();
        assert!(first.load().unwrap().is_none());
    }
    #[cfg(unix)]
    #[test]
    fn permissive_files_and_symlinks_are_rejected() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let temp = tempfile::tempdir().unwrap();
        let store = store(temp.path().join("private"), "https://one.example");
        store.save(&credential("https://one.example")).unwrap();
        assert_eq!(
            fs::metadata(&store.directory).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&store.path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        fs::set_permissions(&store.path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(store.load().is_err());
        assert!(store.save(&credential("https://one.example")).is_err());
        fs::remove_file(&store.path).unwrap();
        let victim = temp.path().join("victim");
        fs::write(&victim, b"unchanged").unwrap();
        symlink(&victim, &store.path).unwrap();
        assert!(store.load().is_err());
        assert!(store.save(&credential("https://one.example")).is_err());
        assert_eq!(fs::read(&victim).unwrap(), b"unchanged");
        let linked = temp.path().join("linked");
        symlink(&store.directory, &linked).unwrap();
        assert!(
            Store::new(
                linked,
                "https://one.example",
                "hireferry-cli",
                StorageChoice::File
            )
            .load()
            .is_err()
        );
    }
    #[tokio::test]
    async fn lock_serializes_writers_and_releases_on_drop() {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("private");
        let first = store(directory.clone(), "https://one.example");
        let guard = first.lock().await.unwrap();
        let (send, mut receive) = tokio::sync::oneshot::channel();
        let worker = tokio::spawn(async move {
            let second = store(directory, "https://one.example");
            let _guard = second.lock().await.unwrap();
            let _ = send.send(());
        });
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut receive)
                .await
                .is_err()
        );
        drop(guard);
        tokio::time::timeout(Duration::from_secs(3), receive)
            .await
            .unwrap()
            .unwrap();
        worker.await.unwrap();
    }

    #[test]
    fn keyring_identity_includes_the_config_directory() {
        let temp = tempfile::tempdir().unwrap();
        let first = store(temp.path().join("first"), "https://one.example");
        let second = store(temp.path().join("second"), "https://one.example");
        first.check_directory(true).unwrap();
        second.check_directory(true).unwrap();
        assert_ne!(first.keyring_key().unwrap(), second.keyring_key().unwrap());
    }

    #[test]
    fn administrator_identity_does_not_require_a_business_grant() {
        let temp = tempfile::tempdir().unwrap();
        let store = store(temp.path().join("private"), "https://one.example");
        let mut credential = credential("https://one.example");
        credential.account = Some(AccountSubject::Administrator);
        store.save(&credential).unwrap();
        let loaded = store.load().unwrap().unwrap();
        assert!(loaded.account == Some(AccountSubject::Administrator));
        assert_eq!(loaded.scope, "identity");
    }
}
