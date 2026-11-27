//! Copying: the clipboard write every copy goes through, and the toast that says what it copied.

use crate::i18n::{tr, trf};
use crate::toast;
use gpui::App;

/// Copies to the clipboard, or into `SUNRISE_LAUNCHER_CLIPBOARD` when a test run sets it
/// (`sunrise_launcher::desktop_seams`).
pub(crate) fn copy_to_clipboard(cx: &mut App, item: gpui::ClipboardItem) {
    match item.text().as_deref().map(copied) {
        Some(Copied::Text(shown)) => toast::say_with("Copied {text}", &shown),
        Some(Copied::Lines(n)) => {
            toast::say(trf("Copied {count} lines", &[("count", &n.to_string())]))
        }
        None => toast::say(tr("Copied the picture")),
    }
    copy_quietly(cx, item);
}

/// The longest copy the toast repeats in full; a longer line ends in an ellipsis.
const COPIED_SHOWN: usize = 64;

/// What the toast says about a copy.
#[derive(Debug, PartialEq)]
enum Copied {
    /// One line, as copied or cut short.
    Text(String),
    /// More than one: a build log would fill the window, so only their count.
    Lines(usize),
}

fn copied(text: &str) -> Copied {
    // Counted as copied: blank lines at the edges are lines too. Only the one-line preview is trimmed.
    let lines = text.lines().count();
    if lines > 1 {
        return Copied::Lines(lines);
    }
    let text = text.trim();
    if text.chars().nth(COPIED_SHOWN).is_none() {
        return Copied::Text(text.to_string());
    }
    let end = text
        .char_indices()
        .nth(COPIED_SHOWN - 1)
        .map_or(text.len(), |(i, _)| i);
    Copied::Text(format!("{}…", text[..end].trim_end()))
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

#[cfg(test)]
mod tests;
