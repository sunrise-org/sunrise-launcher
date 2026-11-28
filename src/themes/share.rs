//! Share codes: one or several themes written as a short code that anyone can paste into
//! their own launcher. There is no server; the code is the data.
//!
//! A code reads `SUN2.` and then base64url (`A-Z a-z 0-9 - _`, six bits a character, no
//! padding and no hyphens to group by) of these bytes:
//!
//! * a flags byte: the layout's version (2) in the low four bits and, in bit 4, whether
//!   everything after this byte is raw-deflate compressed. Compression is only used when it
//!   makes the code shorter, which it rarely does for one theme and often does for many;
//! * the number of themes (1 to 255), then for each theme:
//!   * a two-byte header (big endian): bits 0 to 8 say which of the nine [`SLOTS`] differ from
//!     the theme's reference, bit 9 is 1 for a theme on Sunrise's day look and 0 for its night,
//!     bit 10 picks the reference (0 is Sunrise's palette of the theme's look, 1 is the theme
//!     before it, which the first theme can't pick), and bits 11 to 15 are 0;
//!   * the length of the name in bytes (0 to 32) and the name in UTF-8. An empty name reads as
//!     "Theme 3" for the third;
//!   * three bytes (red, green, blue) for each colour the mask names, in slot order;
//! * the first two bytes of the SHA-256 of the flags byte and what follows it as written (so
//!   before it is inflated), so a mistyped or cut-off code is refused instead of read as other
//!   colours.
//!
//! A theme only costs its differences from the one before it, so a copy of a theme costs its
//! name and two bytes, and themes made from each other stay short. The codes of the first
//! layout, `SUN1-`, are still read, see [`legacy`].
use super::library::{Custom, SLOTS};
use crate::i18n::{tr, trf};
use crate::theme_files::{turns_text, Roles};
use flate2::{read::DeflateDecoder, write::DeflateEncoder, Compression};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};

mod legacy;

/// What every code starts with. The digit is the version of the code's layout.
pub const PREFIX: &str = "SUN2.";
const VERSION: u8 = 2;
/// The flag for a compressed body.
const COMPRESSED: u8 = 0x10;
const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
/// The longest a name is kept, in bytes.
pub const NAME_BYTES: usize = 32;
/// The most themes in one code.
pub const MOST: usize = 255;
/// Bytes of the checksum.
const SUM: usize = 2;
/// The most bytes a body can hold once inflated: 255 themes with the longest name and every
/// colour, and a little more. Anything past it is not a code of ours.
const MOST_BYTES: u64 = 16 * 1024;
/// The header's bit for the day look and for the previous theme as the reference.
const LIGHT_BIT: u16 = 1 << 9;
const PREVIOUS_BIT: u16 = 1 << 10;
const MASK: u16 = (1 << 9) - 1;

/// Why a code can't be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CodeError {
    /// Nothing was pasted.
    Empty,
    /// It doesn't start with `SUN2.`.
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
                "That isn't a theme code. It should start with {prefix}",
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

/// Sunrise's palette on the given look: what a theme is measured from when it doesn't build on
/// the one before.
fn built_in(light: bool) -> Roles {
    if light {
        Roles::DAWN_LIGHT
    } else {
        Roles::DAWN
    }
}

/// A bit for every slot of `roles` that is not what `reference` has.
fn differences(roles: &Roles, reference: &Roles) -> u16 {
    SLOTS.iter().enumerate().fold(0, |mask, (n, slot)| {
        if (slot.get)(roles) == (slot.get)(reference) {
            mask
        } else {
            mask | 1 << n
        }
    })
}

/// The bytes of the themes, before any compression.
fn body_of(themes: &[Custom]) -> Vec<u8> {
    let mut bytes = vec![themes.len() as u8];
    let mut previous: Option<&Roles> = None;
    for theme in themes {
        let roles = &theme.roles;
        let mut header = differences(roles, &built_in(roles.light));
        if let Some(before) = previous {
            // The earlier theme only wins when it saves at least one colour.
            let from_before = differences(roles, before);
            if from_before.count_ones() < header.count_ones() {
                header = from_before | PREVIOUS_BIT;
            }
        }
        let mask = header & MASK;
        header |= if roles.light { LIGHT_BIT } else { 0 };
        bytes.extend_from_slice(&header.to_be_bytes());
        let name = cut(&theme.name, NAME_BYTES);
        bytes.push(name.len() as u8);
        bytes.extend_from_slice(name.as_bytes());
        for (n, slot) in SLOTS.iter().enumerate() {
            if mask >> n & 1 == 1 {
                bytes.extend_from_slice(&(slot.get)(roles).to_be_bytes()[1..]);
            }
        }
        previous = Some(roles);
    }
    bytes
}

fn deflate(raw: &[u8]) -> Option<Vec<u8>> {
    let mut encoder = DeflateEncoder::new(Vec::new(), Compression::best());
    encoder.write_all(raw).ok()?;
    encoder.finish().ok()
}

fn inflate(packed: &[u8]) -> Result<Vec<u8>, CodeError> {
    let mut out = Vec::new();
    DeflateDecoder::new(packed)
        .take(MOST_BYTES + 1)
        .read_to_end(&mut out)
        .map_err(|_| CodeError::Damaged)?;
    if out.len() as u64 > MOST_BYTES {
        return Err(CodeError::Damaged);
    }
    Ok(out)
}

/// `bytes` as base64url, without padding.
fn base64(bytes: &[u8]) -> String {
    let (mut held, mut bits) = (0u32, 0u32);
    let mut text = String::with_capacity(bytes.len() * 4 / 3 + 2);
    for &byte in bytes {
        held = held << 8 | u32::from(byte);
        bits += 8;
        while bits >= 6 {
            bits -= 6;
            text.push(char::from(ALPHABET[(held >> bits & 63) as usize]));
        }
        held &= (1 << bits) - 1;
    }
    if bits > 0 {
        text.push(char::from(ALPHABET[(held << (6 - bits) & 63) as usize]));
    }
    text
}

/// The bytes of base64url text. Bits left over at the end are padding.
fn unbase64(text: &str) -> Result<Vec<u8>, CodeError> {
    let (mut held, mut bits) = (0u32, 0u32);
    let mut bytes = Vec::with_capacity(text.len() * 3 / 4);
    for c in text.trim_end_matches('=').chars() {
        let six = u8::try_from(c)
            .ok()
            .and_then(|b| ALPHABET.iter().position(|&a| a == b))
            .ok_or(CodeError::Characters(c))?;
        held = held << 6 | six as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            bytes.push((held >> bits) as u8);
            held &= (1 << bits) - 1;
        }
    }
    Ok(bytes)
}

/// The flags byte and the body, with their checksum, as a code.
fn armour(compressed: bool, body: &[u8]) -> String {
    let mut bytes = vec![VERSION | if compressed { COMPRESSED } else { 0 }];
    bytes.extend_from_slice(body);
    bytes.extend_from_slice(&checksum(&bytes));
    format!("{PREFIX}{}", base64(&bytes))
}

/// The themes as a code. A name over 32 bytes is cut; past 255 themes, the rest are left out.
pub fn encode(themes: &[Custom]) -> String {
    let raw = body_of(&themes[..themes.len().min(MOST)]);
    match deflate(&raw) {
        Some(packed) if packed.len() < raw.len() => armour(true, &packed),
        _ => armour(false, &raw),
    }
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
/// and one that is not empty. `n` is the theme's place in the code, from 1, for a theme that
/// has no name left.
fn name_of(bytes: &[u8], n: usize) -> Result<String, CodeError> {
    let text = std::str::from_utf8(bytes).map_err(|_| CodeError::Damaged)?;
    let name: String = text
        .chars()
        .filter(|&c| !c.is_control() && !turns_text(c))
        .collect();
    let name = name.trim();
    Ok(if name.is_empty() {
        trf("Theme {n}", &[("n", &n.to_string())])
    } else {
        name.to_string()
    })
}

/// The theme at place `n` (from 1) and what it builds on: `before` is the theme ahead of it.
fn theme_of(reader: &mut Reader, n: usize, before: Option<&Roles>) -> Result<Custom, CodeError> {
    let [high, low] = reader.take(2)? else {
        return Err(CodeError::Truncated);
    };
    let header = u16::from_be_bytes([*high, *low]);
    if header >> 11 != 0 {
        return Err(CodeError::Damaged);
    }
    let len = usize::from(reader.byte()?);
    if len > NAME_BYTES {
        return Err(CodeError::Damaged);
    }
    let name = name_of(reader.take(len)?, n)?;
    let mut roles = if header & PREVIOUS_BIT == 0 {
        built_in(header & LIGHT_BIT != 0)
    } else {
        *before.ok_or(CodeError::Damaged)?
    };
    roles.light = header & LIGHT_BIT != 0;
    for (at, slot) in SLOTS.iter().enumerate() {
        if header >> at & 1 == 1 {
            let [r, g, b] = reader.take(3)? else {
                return Err(CodeError::Truncated);
            };
            (slot.set)(&mut roles, u32::from_be_bytes([0, *r, *g, *b]));
        }
    }
    roles
        .readable()
        .map_err(|_| CodeError::Colours(name.clone()))?;
    Ok(Custom { name, roles })
}

/// The themes in what follows `SUN2.`.
fn decode_body(text: &str) -> Result<Vec<Custom>, CodeError> {
    let bytes = unbase64(text)?;
    if bytes.len() < 1 + SUM {
        return Err(CodeError::Checksum);
    }
    let (data, sum) = bytes.split_at(bytes.len() - SUM);
    if checksum(data) != sum {
        return Err(CodeError::Checksum);
    }
    let flags = data[0];
    if flags & !(0x0f | COMPRESSED) != 0 || flags & 0x0f != VERSION {
        return Err(CodeError::Version);
    }
    let inflated;
    let body = if flags & COMPRESSED != 0 {
        inflated = inflate(&data[1..])?;
        &inflated[..]
    } else {
        &data[1..]
    };
    let mut reader = Reader(body);
    let count = usize::from(reader.byte()?);
    let mut themes: Vec<Custom> = Vec::with_capacity(count);
    for n in 1..=count {
        let before = themes.last().map(|t| t.roles);
        themes.push(theme_of(&mut reader, n, before.as_ref())?);
    }
    if themes.is_empty() || !reader.0.is_empty() {
        return Err(CodeError::Damaged);
    }
    Ok(themes)
}

/// The themes a code holds, in this layout or the first. Spaces and line breaks don't matter,
/// and neither does the case of the prefix.
pub fn decode(code: &str) -> Result<Vec<Custom>, CodeError> {
    let flat: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    if flat.is_empty() {
        return Err(CodeError::Empty);
    }
    if let Some(body) = strip_prefix(&flat, PREFIX) {
        return decode_body(body);
    }
    // The first layout's groups may be hyphenated anywhere, the prefix too.
    let old: String = flat.chars().filter(|&c| c != '-').collect();
    if let Some(body) = strip_prefix(&old, legacy::PREFIX) {
        return legacy::decode(body);
    }
    // SUN3 and on: a layout this launcher doesn't know yet.
    let sun = flat.get(..3).is_some_and(|s| s.eq_ignore_ascii_case("SUN"));
    let newer = matches!(flat.as_bytes().get(3), Some(b'3'..=b'9'));
    Err(if sun && newer {
        CodeError::Version
    } else {
        CodeError::Prefix
    })
}

/// What follows `prefix` in `text`, whatever its case.
fn strip_prefix<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let head = text.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &text[prefix.len()..])
}

#[cfg(test)]
mod tests;
