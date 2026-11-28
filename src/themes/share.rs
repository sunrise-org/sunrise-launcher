//! Share codes: one or several themes written as a short code that anyone can paste into
//! their own launcher. There is no server; the code is the data.
//!
//! A code reads `SUN1-` and then groups of five characters (`SUN1-K7QXR-0M4TC-…`). The groups
//! are base 32, in Crockford's alphabet (no I, L, O or U, so a code survives being read out or
//! retyped; `o` is read as 0 and `i` and `l` as 1), of these bytes:
//!
//! * the format's version (1) and the number of themes;
//! * for each theme: the length of its name in bytes (at most 32), the name in UTF-8, one byte
//!   that is 1 for a theme on Sunrise's day look and 0 for its night, and three bytes (red,
//!   green, blue) for each colour in [`SLOTS`], in order;
//! * the first two bytes of the SHA-256 of all of that, so a mistyped or cut-off code is
//!   refused instead of read as other colours.
//!
//! A theme takes about 40 bytes, so a code for one is about 70 characters. That is too short
//! for compression to help, and it would only make the code harder to follow.
use super::library::{Custom, SLOTS};
use crate::i18n::{tr, trf};
use crate::theme_files::{turns_text, Roles};
use sha2::{Digest, Sha256};

/// What every code starts with. The digit is the version of the code's layout.
pub const PREFIX: &str = "SUN1";
const VERSION: u8 = 1;
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
/// How many characters are between two hyphens.
const GROUP: usize = 5;
/// The longest a name is kept, in bytes.
pub const NAME_BYTES: usize = 32;
/// The most themes in one code.
pub const MOST: usize = 255;
/// Bytes of the checksum.
const SUM: usize = 2;

/// Why a code can't be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CodeError {
    /// Nothing was pasted.
    Empty,
    /// It doesn't start with `SUN1-`.
    Prefix,
    /// A code of another version, from a newer launcher.
    Version,
    /// A character that is in no code.
    Characters(char),
    /// A character was mistyped, or the end is missing.
    Checksum,
    /// The bytes stop before the themes do.
    Truncated,
    /// The bytes are not themes.
    Damaged,
    /// The text of this theme can't be read on its colours.
    Colours(String),
}

impl CodeError {
    /// The reason in the launcher's language.
    pub fn text(&self) -> String {
        match self {
            Self::Empty => tr("Paste a theme code.").into(),
            Self::Prefix => trf(
                "That isn't a theme code. A code starts with {prefix}-.",
                &[("prefix", PREFIX)],
            ),
            Self::Version => tr("This code is from a newer launcher. Update to read it.").into(),
            Self::Characters(c) => trf(
                "A theme code has no {character}. Check what was copied.",
                &[("character", &c.to_string())],
            ),
            Self::Checksum => {
                tr("This code doesn't add up: a character is wrong, or the end is missing.").into()
            }
            Self::Truncated => tr("This code is cut short.").into(),
            Self::Damaged => tr("This code can't be read as themes.").into(),
            Self::Colours(name) => trf(
                "{name}'s text can't be read on its colours.",
                &[("name", name)],
            ),
        }
    }
}

/// `name` cut to at most `max` bytes, never in the middle of a character.
fn cut(name: &str, max: usize) -> &str {
    let mut end = name.len().min(max);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    &name[..end]
}

fn checksum(bytes: &[u8]) -> [u8; SUM] {
    let digest = Sha256::digest(bytes);
    [digest[0], digest[1]]
}

/// `bytes` and their checksum, written as a code.
fn armour(mut bytes: Vec<u8>) -> String {
    bytes.extend_from_slice(&checksum(&bytes));
    let (mut held, mut bits) = (0u32, 0u32);
    let mut body = String::new();
    for byte in bytes {
        held = held << 8 | u32::from(byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            body.push(char::from(ALPHABET[(held >> bits & 31) as usize]));
        }
        held &= (1 << bits) - 1;
    }
    if bits > 0 {
        body.push(char::from(ALPHABET[(held << (5 - bits) & 31) as usize]));
    }
    let mut code = String::from(PREFIX);
    for (n, c) in body.chars().enumerate() {
        if n % GROUP == 0 {
            code.push('-');
        }
        code.push(c);
    }
    code
}

/// The themes as a code. A name over 32 bytes is cut; past 255 themes, the rest are left out.
pub fn encode(themes: &[Custom]) -> String {
    let themes = &themes[..themes.len().min(MOST)];
    let mut bytes = vec![VERSION, themes.len() as u8];
    for theme in themes {
        let name = cut(&theme.name, NAME_BYTES);
        bytes.push(name.len() as u8);
        bytes.extend_from_slice(name.as_bytes());
        bytes.push(u8::from(theme.roles.light));
        for slot in &SLOTS {
            bytes.extend_from_slice(&(slot.get)(&theme.roles).to_be_bytes()[1..]);
        }
    }
    armour(bytes)
}

fn digit(c: char) -> Result<u32, CodeError> {
    let c = match c.to_ascii_uppercase() {
        'O' => '0',
        'I' | 'L' => '1',
        other => other,
    };
    ALPHABET
        .iter()
        .position(|&a| char::from(a) == c)
        .map(|n| n as u32)
        .ok_or(CodeError::Characters(c))
}

/// The bytes a code's groups hold: the checksum is still in them.
fn unarmour(body: &str) -> Result<Vec<u8>, CodeError> {
    let (mut held, mut bits) = (0u32, 0u32);
    let mut bytes = Vec::new();
    for c in body.chars().filter(|&c| c != '-') {
        held = held << 5 | digit(c)?;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            bytes.push((held >> bits) as u8);
            held &= (1 << bits) - 1;
        }
    }
    Ok(bytes)
}

/// Reads bytes front to back.
struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], CodeError> {
        if self.0.len() < n {
            return Err(CodeError::Truncated);
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Ok(head)
    }

    fn byte(&mut self) -> Result<u8, CodeError> {
        Ok(self.take(1)?[0])
    }
}

/// A name as the library keeps it: no control characters, nothing that turns the text around,
/// and one that is not empty.
fn name_of(bytes: &[u8]) -> Result<String, CodeError> {
    let text = std::str::from_utf8(bytes).map_err(|_| CodeError::Damaged)?;
    let name: String = text
        .chars()
        .filter(|&c| !c.is_control() && !turns_text(c))
        .collect();
    let name = name.trim();
    Ok(if name.is_empty() {
        tr("Theme").to_string()
    } else {
        name.to_string()
    })
}

fn theme_of(reader: &mut Reader) -> Result<Custom, CodeError> {
    let len = usize::from(reader.byte()?);
    let name = name_of(reader.take(len)?)?;
    let mut roles = Roles::DAWN;
    roles.light = match reader.byte()? {
        0 => false,
        1 => true,
        _ => return Err(CodeError::Damaged),
    };
    for slot in &SLOTS {
        let [r, g, b] = reader.take(3)? else {
            return Err(CodeError::Truncated);
        };
        (slot.set)(&mut roles, u32::from_be_bytes([0, *r, *g, *b]));
    }
    roles
        .readable()
        .map_err(|_| CodeError::Colours(name.clone()))?;
    Ok(Custom { name, roles })
}

/// The themes a code holds. Spaces, line breaks and case don't matter, and neither does where
/// the hyphens are.
pub fn decode(code: &str) -> Result<Vec<Custom>, CodeError> {
    let flat: String = code
        .chars()
        .filter(|&c| !c.is_whitespace() && c != '-')
        .collect();
    if flat.is_empty() {
        return Err(CodeError::Empty);
    }
    let body = match flat.get(..PREFIX.len()) {
        Some(head) if head.eq_ignore_ascii_case(PREFIX) => &flat[PREFIX.len()..],
        _ => {
            // SUN2 and on: a layout this launcher doesn't know yet.
            let sun = flat.get(..3).is_some_and(|s| s.eq_ignore_ascii_case("SUN"));
            let newer = matches!(flat.as_bytes().get(3), Some(b'2'..=b'9'));
            return Err(if sun && newer {
                CodeError::Version
            } else {
                CodeError::Prefix
            });
        }
    };
    let bytes = unarmour(body)?;
    if bytes.len() < SUM {
        return Err(CodeError::Checksum);
    }
    let (data, sum) = bytes.split_at(bytes.len() - SUM);
    if checksum(data) != sum {
        return Err(CodeError::Checksum);
    }
    let mut reader = Reader(data);
    if reader.byte()? != VERSION {
        return Err(CodeError::Version);
    }
    let count = reader.byte()?;
    let themes = (0..count)
        .map(|_| theme_of(&mut reader))
        .collect::<Result<Vec<_>, _>>()?;
    if themes.is_empty() || !reader.0.is_empty() {
        return Err(CodeError::Damaged);
    }
    Ok(themes)
}

#[cfg(test)]
mod tests;
