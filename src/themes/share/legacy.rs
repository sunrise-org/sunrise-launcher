//! The first layout, `SUN1-`, which launchers wrote until the shorter one came. Codes in it are
//! still in chats and notes, so they are still read; nothing writes them any more.
//!
//! A code reads `SUN1-` and then groups of five characters (`SUN1-K7QXR-0M4TC-…`). The groups
//! are base 32, in Crockford's alphabet (no I, L, O or U; `o` is read as 0 and `i` and `l` as
//! 1), of these bytes:
//!
//! * the layout's version (1) and the number of themes;
//! * for each theme: the length of its name in bytes (at most 32), the name in UTF-8, one byte
//!   that is 1 for a theme on Sunrise's day look and 0 for its night, and three bytes (red,
//!   green, blue) for each colour in [`SLOTS`], in order;
//! * the first two bytes of the SHA-256 of all of that.
use super::{checksum, name_of, CodeError, Reader, SUM};
use crate::theme_files::Roles;
use crate::themes::library::{Custom, SLOTS};

pub(super) const PREFIX: &str = "SUN1";
const VERSION: u8 = 1;
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

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

/// `bytes` and their checksum, written the way the first layout did. Only the tests write
/// codes this way, to check that they still read.
#[cfg(test)]
pub(super) fn armour(mut bytes: Vec<u8>) -> String {
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
        if n % 5 == 0 {
            code.push('-');
        }
        code.push(c);
    }
    code
}

fn theme_of(reader: &mut Reader, n: usize) -> Result<Custom, CodeError> {
    let len = usize::from(reader.byte()?);
    let name = name_of(reader.take(len)?, n)?;
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

/// The themes in what follows `SUN1`.
pub(super) fn decode(body: &str) -> Result<Vec<Custom>, CodeError> {
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
        .map(|n| theme_of(&mut reader, usize::from(n) + 1))
        .collect::<Result<Vec<_>, _>>()?;
    if themes.is_empty() || !reader.0.is_empty() {
        return Err(CodeError::Damaged);
    }
    Ok(themes)
}
