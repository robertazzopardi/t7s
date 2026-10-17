use std::str::FromStr;

use crate::{
    auth::Keyring,
    db::connection::Connection,
    ui::widgets::connection_modal::{PasswordStorageType, TestResult},
};

/// Handles saving a connection from the modal
///
/// # Errors
///
/// Returns an error string if:
/// - Keyring creation fails
/// - Keyring is locked
/// - Password storage in keyring fails
/// - Database save/update operations fail
#[allow(dead_code, clippy::ref_option)]
pub fn handle_save_connection(
    #[allow(unused_variables)] keyring: &Option<Keyring>,
    connection: &Connection,
    mode: crate::ui::widgets::connection_modal::Mode,
    original_name: Option<String>,
) -> Result<(), String> {
    // Handle password storage based on connection's storage preference
    // Only save to keyring if password_storage is Keyring (not "ask every time")
    #[allow(unused_variables)]
    if let Some(password) = &connection.password {
        let storage_type = connection
            .password_storage
            .as_ref()
            .map_or(PasswordStorageType::Keyring, |s| {
                PasswordStorageType::from_str(s).unwrap_or_default()
            });

        match storage_type {
            PasswordStorageType::Keyring => {
                // Always save to keyring when password_storage is Keyring
                // In dev mode, don't actually save to keyring (but UI still shows the option)
                #[cfg(not(debug_assertions))]
                {
                    // Never create a new keyring here - it should be created in app.rs before calling this
                    if let Some(keyring) = keyring {
                        let keyring_result = keyring.set_password(password);

                        if let Err(e) = keyring_result {
                            let error_msg = e.to_string();
                            if error_msg.contains("locked collection") {
                                return Err(
                                    "Keyring is locked. Please unlock your keyring first.\n\n\
                                    On Linux, you can unlock it using:\n\
                                    - seahorse (GUI: search for 'Passwords and Keys')\n\
                                    - Or unlock it when prompted by your desktop environment\n\n\
                                    Alternatively, you can save the connection without storing the password in the keyring."
                                        .to_string(),
                                );
                            }
                            return Err(format!(
                                "Failed to store password in keyring: {error_msg}\n\n\
                                Hint: If your keyring is locked, unlock it first using your system's keyring manager."
                            ));
                        }
                    }
                }
                #[cfg(debug_assertions)]
                {
                    // In dev mode, passwords are not saved to keyring
                    // The preference is still stored in the database for consistency
                }
            }
            PasswordStorageType::DontSave => {
                // Don't save password - connection will work but password won't be stored
                // Never access keyring for "ask every time" connections
            }
        }
    }

    if matches!(mode, crate::ui::widgets::connection_modal::Mode::New) {
        crate::db::sqlite::save_connection(connection)
            .map_err(|e| format!("Failed to save connection: {e}"))?;
    } else if matches!(mode, crate::ui::widgets::connection_modal::Mode::Edit)
        && let Some(original_name) = original_name
    {
        crate::db::sqlite::update_connection(&original_name, connection)
            .map_err(|e| format!("Failed to update connection: {e}"))?;

        // Delete the old credential if the name has changed
        if original_name != connection.name
            && let Ok(keyring) = Keyring::new(&original_name)
        {
            let _ = keyring.delete_password();
        }
    }

    Ok(())
}

/// Tests a database connection (Postgres or Sqlite)
#[allow(dead_code)]
pub async fn test_connection(connection: &Connection) -> TestResult {
    let result = connection.to_database().test().await;

    if result {
        TestResult::Success
    } else {
        TestResult::Failed("Connection failed".to_string())
    }
}
