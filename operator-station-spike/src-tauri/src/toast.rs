use crate::model::StationMessage;
use std::path::{Path, PathBuf};

pub const AUMID: &str = "com.lossyrob.telex.operatorstationspike";
const APP_ICON: &[u8] = include_bytes!("../../../assets/telex.png");

pub fn prepare(app_data_dir: &Path) -> Result<PathBuf, String> {
    let icon_path = app_data_dir.join("operator-station-spike.png");
    std::fs::create_dir_all(app_data_dir)
        .map_err(|error| format!("creating toast app-data directory failed: {error}"))?;
    std::fs::write(&icon_path, APP_ICON)
        .map_err(|error| format!("writing toast icon failed: {error}"))?;
    register(&icon_path)?;
    Ok(icon_path)
}

#[cfg(target_os = "windows")]
pub fn set_process_aumid() {
    use windows::core::HSTRING;
    use windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;
    if let Err(error) = unsafe { SetCurrentProcessExplicitAppUserModelID(&HSTRING::from(AUMID)) } {
        eprintln!("operator-station-spike: setting AUMID failed: {error}");
    }
}

#[cfg(not(target_os = "windows"))]
pub fn set_process_aumid() {}

#[cfg(target_os = "windows")]
fn register(icon_path: &Path) -> Result<(), String> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let (key, _) = hkcu
        .create_subkey(format!(r"Software\Classes\AppUserModelId\{AUMID}"))
        .map_err(|error| format!("creating AUMID registry key failed: {error}"))?;
    key.set_value("DisplayName", &"Telex Operator Station Spike")
        .map_err(|error| format!("writing AUMID DisplayName failed: {error}"))?;
    key.set_value("IconUri", &icon_path.display().to_string())
        .map_err(|error| format!("writing AUMID IconUri failed: {error}"))?;
    key.set_value("IconBackgroundColor", &"0")
        .map_err(|error| format!("writing AUMID icon background failed: {error}"))?;
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn register(_icon_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(target_os = "windows")]
pub fn show(message: &StationMessage) -> Result<(), String> {
    use windows::core::HSTRING;
    use windows::Data::Xml::Dom::XmlDocument;
    use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};

    let title = message
        .subject
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("Telex operator attention");
    let attribution = message.from.as_deref().unwrap_or("Telex");
    let xml = format!(
        "<toast><visual><binding template=\"ToastGeneric\"><text>{}</text><text>{}</text><text placement=\"attribution\">{}</text></binding></visual></toast>",
        xml_escape(title),
        xml_escape(&message.body),
        xml_escape(attribution),
    );
    let document =
        XmlDocument::new().map_err(|error| format!("creating toast XML failed: {error}"))?;
    document
        .LoadXml(&HSTRING::from(xml))
        .map_err(|error| format!("loading toast XML failed: {error}"))?;
    let toast = ToastNotification::CreateToastNotification(&document)
        .map_err(|error| format!("creating toast failed: {error}"))?;
    let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(AUMID))
        .map_err(|error| format!("creating toast notifier failed: {error}"))?;
    notifier
        .Show(&toast)
        .map_err(|error| format!("showing toast failed: {error}"))
}

#[cfg(not(target_os = "windows"))]
pub fn show(_message: &StationMessage) -> Result<(), String> {
    Ok(())
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toast_text_is_xml_escaped() {
        assert_eq!(
            xml_escape("<operator & \"human\">"),
            "&lt;operator &amp; &quot;human&quot;&gt;"
        );
    }
}
