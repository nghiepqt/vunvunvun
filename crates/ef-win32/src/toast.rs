/// Result of a best-effort Windows Toast notification dispatch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToastResult {
    Shown,
    Unavailable(String),
}

/// Escapes special XML characters in user-visible toast copy.
#[must_use]
pub fn escape_xml(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
    out
}

/// Builds a ToastGeneric XML payload with plain escaped text.
#[must_use]
pub fn build_toast_xml(title: &str, body: &str) -> String {
    format!(
        "<toast><visual><binding template=\"ToastGeneric\"><text>{}</text><text>{}</text></binding></visual></toast>",
        escape_xml(title),
        escape_xml(body)
    )
}

/// Sends a best-effort Windows Toast notification.
///
/// Focus Assist / Do Not Disturb or missing unpackaged AUMID shortcuts can suppress
/// delivery silently (`PHASE-02` §9), so callers must always keep an in-app path.
#[must_use]
pub fn show_toast_best_effort(title: &str, body: &str, is_packaged: bool) -> ToastResult {
    #[cfg(windows)]
    {
        windows_impl::show_toast(title, body, is_packaged)
    }
    #[cfg(not(windows))]
    {
        let _ = (title, body, is_packaged);
        ToastResult::Unavailable("toasts are only available on Windows".to_string())
    }
}

#[cfg(windows)]
mod windows_impl {
    use windows::Data::Xml::Dom::XmlDocument;
    use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};
    use windows::core::HSTRING;

    use super::{ToastResult, build_toast_xml};

    const APP_AUMID: &str = "ThanhNghiepQuan.Vun";

    pub(super) fn show_toast(title: &str, body: &str, is_packaged: bool) -> ToastResult {
        let xml_str = build_toast_xml(title, body);
        let Ok(doc) = XmlDocument::new() else {
            return ToastResult::Unavailable("XmlDocument::new failed".to_string());
        };
        if let Err(err) = doc.LoadXml(&HSTRING::from(xml_str)) {
            return ToastResult::Unavailable(format!("XmlDocument::LoadXml failed: {err}"));
        }
        let Ok(toast) = ToastNotification::CreateToastNotification(&doc) else {
            return ToastResult::Unavailable("CreateToastNotification failed".to_string());
        };
        let notifier_res = if is_packaged {
            ToastNotificationManager::CreateToastNotifier()
        } else {
            ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(APP_AUMID))
        };
        let notifier = match notifier_res {
            Ok(n) => n,
            Err(err) => {
                return ToastResult::Unavailable(format!("CreateToastNotifier failed: {err}"));
            }
        };
        match notifier.Show(&toast) {
            Ok(()) => ToastResult::Shown,
            Err(err) => ToastResult::Unavailable(format!("ToastNotifier::Show failed: {err}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toast_xml_escapes_special_chars_and_vietnamese() {
        let xml = build_toast_xml("Vun & <Nhắc>", "Phím \"Ctrl+Shift+Space\" đã dùng 'khác'");
        assert!(xml.contains("&amp; &lt;Nhắc&gt;"));
        assert!(xml.contains("&quot;Ctrl+Shift+Space&quot;"));
        assert!(xml.contains("&apos;khác&apos;"));
    }
}
