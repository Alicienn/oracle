//! Starting Oracle when Windows starts.
//!
//! Written directly to `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` rather than
//! through a scheduled task or the Startup folder. The registry key needs no elevation, is
//! what the Settings app shows the user under "Startup apps", and can be removed by the user
//! outside Oracle without leaving anything behind.

#[cfg(windows)]
const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

/// The name Oracle registers itself under.
#[cfg(windows)]
const VALUE_NAME: &str = "Oracle";

/// Passed on an autostarted launch so the app knows to come up in the tray.
pub const HIDDEN_FLAG: &str = "--hidden";

#[cfg(windows)]
pub fn is_enabled() -> bool {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};
    use winreg::RegKey;

    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(RUN_KEY, KEY_READ)
        .and_then(|key| key.get_value::<String, _>(VALUE_NAME))
        .is_ok()
}

/// Registers or unregisters Oracle.
///
/// `start_hidden` decides whether the autostarted instance shows its window or only its
/// tray icon — the usual choice for something meant to sit in the background.
#[cfg(windows)]
pub fn set(enabled: bool, start_hidden: bool) -> crate::error::Result<()> {
    use crate::error::OracleError;
    use winreg::enums::{HKEY_CURRENT_USER, KEY_WRITE};
    use winreg::RegKey;

    let key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(RUN_KEY, KEY_WRITE)
        .map_err(|err| OracleError::Other(format!("cannot open the startup registry key: {err}")))?;

    if !enabled {
        // Deleting a value that is not there is success, not failure.
        match key.delete_value(VALUE_NAME) {
            Ok(()) => return Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(err) => {
                return Err(OracleError::Other(format!(
                    "cannot clear the startup entry: {err}"
                )))
            }
        }
    }

    let exe = std::env::current_exe()
        .map_err(|err| OracleError::Other(format!("cannot locate the Oracle executable: {err}")))?;

    let command = command_line(&exe.to_string_lossy(), start_hidden);

    key.set_value(VALUE_NAME, &command)
        .map_err(|err| OracleError::Other(format!("cannot write the startup entry: {err}")))
}

/// Builds the registry command string.
///
/// The path is quoted because `Program Files` and the user's own name routinely contain
/// spaces, and an unquoted path there launches the wrong thing or nothing at all.
fn command_line(exe: &str, start_hidden: bool) -> String {
    if start_hidden {
        format!("\"{exe}\" {HIDDEN_FLAG}")
    } else {
        format!("\"{exe}\"")
    }
}

/// True when this process was launched by the autostart entry.
pub fn launched_hidden() -> bool {
    std::env::args().any(|arg| arg == HIDDEN_FLAG)
}

#[cfg(not(windows))]
pub fn is_enabled() -> bool {
    false
}

#[cfg(not(windows))]
pub fn set(_enabled: bool, _start_hidden: bool) -> crate::error::Result<()> {
    Err(crate::error::OracleError::Other(
        "starting with the system is only supported on Windows".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_executable_path_is_always_quoted() {
        let line = command_line(r"C:\Program Files\Oracle\oracle.exe", false);
        assert_eq!(line, "\"C:\\Program Files\\Oracle\\oracle.exe\"");
    }

    #[test]
    fn the_hidden_flag_is_appended_outside_the_quotes() {
        let line = command_line(r"C:\Apps\oracle.exe", true);
        assert_eq!(line, "\"C:\\Apps\\oracle.exe\" --hidden");
    }

    #[test]
    fn a_path_with_spaces_survives_quoting() {
        let line = command_line(r"C:\Users\Jean Dupont\oracle.exe", true);
        assert!(line.starts_with("\"C:\\Users\\Jean Dupont\\oracle.exe\""));
        assert!(line.ends_with(HIDDEN_FLAG));
    }
}
