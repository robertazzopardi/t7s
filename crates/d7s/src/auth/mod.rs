use core::{
    error::Error as StdError,
    fmt::{Display, Formatter, Result as FmtResult},
};
#[cfg(not(debug_assertions))]
use std::sync::OnceLock;
#[cfg(debug_assertions)]
use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
};

#[cfg(not(debug_assertions))]
use keyring_core::Entry;

#[cfg(debug_assertions)]
#[allow(dead_code)]
mod keep_linked {
    // Debug builds use the in-memory store; keep platform crates linked.
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    use apple_native_keyring_store as _;
    #[cfg(all(
        unix,
        not(any(
            target_os = "macos",
            target_os = "ios",
            target_os = "android"
        ))
    ))]
    use dbus_secret_service_keyring_store as _;
    use keyring_core as _;
    #[cfg(target_os = "windows")]
    use windows_native_keyring_store as _;
}

#[cfg(not(debug_assertions))]
const SERVICE_NAME: &str = "d7s";

#[cfg(not(debug_assertions))]
pub struct Keyring {
    entry: Entry,
}

#[cfg(debug_assertions)]
pub struct Keyring {
    user: String,
}

#[cfg(debug_assertions)]
fn dev_store() -> &'static Mutex<HashMap<String, String>> {
    static STORE: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

#[cfg(not(debug_assertions))]
#[derive(Debug)]
pub enum Error {
    KeyringError(keyring_core::Error),
}

#[cfg(not(debug_assertions))]
impl From<keyring_core::Error> for Error {
    fn from(err: keyring_core::Error) -> Self {
        Self::KeyringError(err)
    }
}

#[cfg(debug_assertions)]
#[derive(Debug)]
pub enum Error {
    NotFound,
    Other(String),
}

#[cfg(debug_assertions)]
impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            Self::NotFound => write!(f, "Password not found"),
            Self::Other(msg) => write!(f, "{msg}"),
        }
    }
}

#[cfg(debug_assertions)]
impl StdError for Error {}

#[cfg(not(debug_assertions))]
impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> FmtResult {
        match self {
            Self::KeyringError(e) => write!(f, "{e}"),
        }
    }
}

#[cfg(not(debug_assertions))]
impl StdError for Error {}

#[cfg(not(debug_assertions))]
fn init_platform_store() -> keyring_core::Result<()> {
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    {
        let store = apple_native_keyring_store::keychain::Store::new()?;
        keyring_core::set_default_store(store);
        return Ok(());
    }
    #[cfg(target_os = "windows")]
    {
        let store = windows_native_keyring_store::Store::new()?;
        keyring_core::set_default_store(store);
        return Ok(());
    }
    #[cfg(all(
        unix,
        not(any(
            target_os = "macos",
            target_os = "ios",
            target_os = "android"
        ))
    ))]
    {
        let store = dbus_secret_service_keyring_store::Store::new()?;
        keyring_core::set_default_store(store);
        return Ok(());
    }
    #[cfg(not(all(
        any(unix, windows),
        not(any(target_os = "ios", target_os = "android"))
    )))]
    {
        return Err(keyring_core::Error::Invalid(
            "platform".to_string(),
            "must be macOS, Windows, or a non-iOS, non-Android *nix variant"
                .to_string(),
        ));
    }
}

/// `Ok` if the default store is ready; `Err` if init failed (`Error` is not `Clone`).
#[cfg(not(debug_assertions))]
fn ensure_default_store() -> Result<(), Error> {
    static INIT: OnceLock<Result<(), ()>> = OnceLock::new();
    match INIT.get_or_init(|| {
        init_platform_store().map_err(|err| {
            // OnceLock can't keep the platform error; print once for headless debug.
            eprintln!("d7s keyring store init failed: {err}");
        })
    }) {
        Ok(()) => Ok(()),
        Err(()) => {
            Err(Error::KeyringError(keyring_core::Error::NoDefaultStore))
        }
    }
}

impl Keyring {
    /// Creates a new keyring entry
    ///
    /// # Errors
    ///
    /// Returns an error if the keyring entry cannot be created
    #[cfg(not(debug_assertions))]
    pub fn new(user: &str) -> Result<Self, Error> {
        ensure_default_store()?;
        Ok(Self {
            entry: Entry::new(SERVICE_NAME, user)?,
        })
    }

    /// Creates a new in-memory credential store (dev mode)
    ///
    /// # Errors
    ///
    /// Never returns an error in dev mode
    #[cfg(debug_assertions)]
    #[allow(clippy::unnecessary_wraps)]
    pub fn new(user: &str) -> Result<Self, Error> {
        Ok(Self {
            user: user.to_string(),
        })
    }

    /// Sets the password in the keyring
    ///
    /// # Errors
    ///
    /// Returns an error if the password cannot be set
    #[cfg(not(debug_assertions))]
    pub fn set_password(&self, password: &str) -> Result<(), Error> {
        self.entry.set_password(password).map_err(Error::from)
    }

    /// Sets the password in the in-memory store (dev mode)
    ///
    /// # Errors
    ///
    /// Never returns an error in dev mode
    #[cfg(debug_assertions)]
    pub fn set_password(&self, password: &str) -> Result<(), Error> {
        dev_store()
            .lock()
            .map_err(|err| Error::Other(err.to_string()))?
            .insert(self.user.clone(), password.to_string());
        Ok(())
    }

    /// Gets the password from the keyring
    ///
    /// # Errors
    ///
    /// Returns an error if the password cannot be retrieved
    #[cfg(not(debug_assertions))]
    pub fn get_password(&self) -> Result<String, Error> {
        self.entry.get_password().map_err(Error::from)
    }

    /// Gets the password from the in-memory store (dev mode)
    ///
    /// # Errors
    ///
    /// Returns an error if the password is not found
    #[cfg(debug_assertions)]
    pub fn get_password(&self) -> Result<String, Error> {
        dev_store()
            .lock()
            .map_err(|err| Error::Other(err.to_string()))?
            .get(&self.user)
            .cloned()
            .ok_or(Error::NotFound)
    }

    /// Deletes the password from the keyring
    ///
    /// # Errors
    ///
    /// Returns an error if the password cannot be deleted
    #[cfg(not(debug_assertions))]
    pub fn delete_password(&self) -> Result<(), Error> {
        self.entry.delete_credential().map_err(Error::from)
    }

    /// Deletes the password from the in-memory store (dev mode)
    ///
    /// # Errors
    ///
    /// Never returns an error in dev mode
    #[cfg(debug_assertions)]
    pub fn delete_password(&self) -> Result<(), Error> {
        dev_store()
            .lock()
            .map_err(|err| Error::Other(err.to_string()))?
            .remove(&self.user);
        Ok(())
    }
}
