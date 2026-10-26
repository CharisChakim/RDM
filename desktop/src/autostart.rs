//! "Start with Windows": an HKCU Run entry that launches RDM straight into the tray.

use winreg::enums::{HKEY_CURRENT_USER, KEY_SET_VALUE};
use winreg::RegKey;

const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const NAME: &str = "RDM";

pub fn apply(on: bool) {
    let Ok(run) = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(RUN, KEY_SET_VALUE) else { return };
    if on {
        if let Ok(exe) = std::env::current_exe() {
            let _ = run.set_value(NAME, &format!("\"{}\" --hidden", exe.display()));
        }
    } else {
        let _ = run.delete_value(NAME);
    }
}
