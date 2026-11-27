//! Copying: the clipboard write every copy goes through, and the toast that says what it copied.

use crate::i18n::tr;
use crate::toast;
use gpui::App;

/// Copies to the clipboard, or into `SUNRISE_LAUNCHER_CLIPBOARD` when a test run sets it
/// (`sunrise_launcher::desktop_seams`).
pub(crate) fn copy_to_clipboard(cx: &mut App, item: gpui::ClipboardItem) {
    match item.text() {
        Some(text) => toast::copied(&text),
        None => toast::say(tr("Copied the picture")),
    }
    copy_quietly(cx, item);
}

/// Copies like [`copy_to_clipboard`] without its toast, for a button that shows the copy itself.
pub(crate) fn copy_quietly(cx: &mut App, item: gpui::ClipboardItem) {
    if let Some(file) = sunrise_launcher::desktop_seams::clipboard_file() {
        let bytes = match item.entries().first() {
            Some(gpui::ClipboardEntry::Image(image)) => image.bytes.clone(),
            _ => item.text().unwrap_or_default().into_bytes(),
        };
        if let Err(e) = std::fs::write(&file, bytes) {
            eprintln!("{}: {e}", file.display());
        }
        return;
    }
    cx.write_to_clipboard(item);
}
