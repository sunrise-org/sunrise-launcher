use super::legacy::{armour as armour_sun1, PREFIX as SUN1};
use super::*;
use crate::themes::library::ACCENT;

/// Sunrise's night, written out so the tests don't move with the built-in palettes.
const NIGHT: Roles = Roles {
    background: 0x2b2324,
    card: 0x322829,
    chrome: 0x1e1b1a,
    text: 0xf3eae6,
    muted: 0xb9aba6,
    accent: 0xc2552d,
    line: 0xf3eae6,
    success: 0x7dc98c,
    danger: 0xec8d8d,
    light: false,
};

const DAY: Roles = Roles {
    background: 0xf7f1ee,
    card: 0xffffff,
    chrome: 0xefe7e3,
    text: 0x2b2324,
    muted: 0x6f625e,
    accent: 0xc2552d,
    line: 0xd9cdc8,
    success: 0x23703a,
    danger: 0xb93a3a,
    light: true,
};

fn theme(name: &str, roles: Roles) -> Custom {
    Custom {
        name: name.into(),
        roles,
    }
}

/// A code with any flags byte and any body, with the right checksum.
fn code_of(flags: u8, body: &[u8]) -> String {
    let mut bytes = vec![flags];
    bytes.extend_from_slice(body);
    bytes.extend_from_slice(&checksum(&bytes));
    format!("{PREFIX}{}", base64(&bytes))
}

/// The bytes of a code, without its checksum.
fn bytes_of(code: &str) -> Vec<u8> {
    let mut bytes = unbase64(&code[PREFIX.len()..]).unwrap();
    bytes.truncate(bytes.len() - SUM);
    bytes
}

fn tweaked(n: u32, base: Roles) -> Roles {
    Roles {
        accent: base.accent - (n << 16),
        ..base
    }
}

fn round_trips(themes: Vec<Custom>) {
    let code = encode(&themes);
    assert!(code.starts_with(PREFIX), "{code}");
    assert_eq!(decode(&code), Ok(themes));
}

#[test]
fn a_code_of_the_first_layout_still_reads() {
    // Codes in this layout are in chats and notes: it must keep reading.
    let code = "SUN1-040GA-HBDC9-JQ401-B4CJ1-W6RT6-8M2KW-ZAWTW-TQ9P2-AMPZ7-TQ6FQ-4RSV4-DHN5S-E";
    assert_eq!(decode(code), Ok(vec![theme("Ember", NIGHT)]));
    // However it was typed: lower case, spaces, line breaks, hyphens anywhere or none.
    let loose = code.to_lowercase().replace('-', " \n");
    assert_eq!(decode(&loose), Ok(vec![theme("Ember", NIGHT)]));
    assert_eq!(
        decode(&code.replace('-', "")),
        Ok(vec![theme("Ember", NIGHT)])
    );
    // O reads as 0 and I and L as 1.
    let misread = code
        .replace('0', "O")
        .replace('1', "l")
        .replace("SUNl", "SUN1");
    assert_eq!(decode(&misread), Ok(vec![theme("Ember", NIGHT)]));
    // Several themes, written the way the first layout wrote them.
    let mut bytes = vec![1, 2];
    for (name, roles) in [("Ember", NIGHT), ("Paper", DAY)] {
        bytes.push(name.len() as u8);
        bytes.extend_from_slice(name.as_bytes());
        bytes.push(u8::from(roles.light));
        for slot in &SLOTS {
            bytes.extend_from_slice(&(slot.get)(&roles).to_be_bytes()[1..]);
        }
    }
    assert_eq!(
        decode(&armour_sun1(bytes)),
        Ok(vec![theme("Ember", NIGHT), theme("Paper", DAY)])
    );
}

#[test]
fn the_first_layouts_errors_still_hold() {
    let code = "SUN1-040GA-HBDC9-JQ401-B4CJ1-W6RT6-8M2KW-ZAWTW-TQ9P2-AMPZ7-TQ6FQ-4RSV4-DHN5S-E";
    let mut chars: Vec<char> = code.chars().collect();
    chars[20] = if chars[20] == 'A' { 'B' } else { 'A' };
    assert_eq!(
        decode(&chars.into_iter().collect::<String>()),
        Err(CodeError::Checksum)
    );
    assert_eq!(decode(&code[..code.len() - 8]), Err(CodeError::Checksum));
    assert_eq!(decode("SUN1-ABCDE-U!"), Err(CodeError::Characters('U')));
    assert_eq!(decode("SUN1-"), Err(CodeError::Checksum));
    assert_eq!(
        decode(&armour_sun1(vec![1, 2, 3])),
        Err(CodeError::Truncated)
    );
    assert_eq!(decode(&armour_sun1(vec![1, 0])), Err(CodeError::Damaged));
    assert_eq!(decode(&armour_sun1(vec![2, 0])), Err(CodeError::Version));
    let mut bytes = vec![1, 1, 1, b'x', 9];
    bytes.extend([0x10; 27]);
    assert_eq!(decode(&armour_sun1(bytes)), Err(CodeError::Damaged));
    assert!(code.starts_with(SUN1));
}

#[test]
fn a_code_is_pinned_so_it_keeps_reading() {
    // Hand-checked: flags 02, one theme, header 0000 (Sunrise's night, nothing changed), the
    // name "Ember", two bytes of checksum.
    let plain = "SUN2.AgEAAAVFbWJlcieu";
    assert_eq!(encode(&[theme("Ember", Roles::DAWN)]), plain);
    assert_eq!(decode(plain), Ok(vec![theme("Ember", Roles::DAWN)]));
    // The same with the accent changed: header 0020, then 11 22 33.
    let accent = "SUN2.AgEAIAVFbWJlchEiM94y";
    let roles = Roles {
        accent: 0x112233,
        ..Roles::DAWN
    };
    assert_eq!(ACCENT, 5);
    assert_eq!(encode(&[theme("Ember", roles)]), accent);
    assert_eq!(decode(accent), Ok(vec![theme("Ember", roles)]));
}

#[test]
fn themes_of_every_kind_survive_the_trip() {
    round_trips(vec![theme("Ember", NIGHT)]);
    round_trips(vec![theme("Paper", DAY)]);
    round_trips(vec![theme("Ember", NIGHT), theme("Paper", DAY)]);
    // Light and dark mixed, in both orders, and one built on the one before.
    round_trips(vec![
        theme("Paper", DAY),
        theme("Ember", NIGHT),
        theme(
            "Ember Two",
            Roles {
                accent: 0x112233,
                ..NIGHT
            },
        ),
        theme("Paper Two", tweaked(3, DAY)),
    ]);
    // Identical themes, and Sunrise's own palettes.
    round_trips(vec![theme("Ember", NIGHT); 3]);
    round_trips(vec![
        theme("Night", Roles::DAWN),
        theme("Day", Roles::DAWN_LIGHT),
        theme("Night again", Roles::DAWN),
    ]);
    // Ten, one built on the one before and the look swapping.
    let ten: Vec<Custom> = (0..10)
        .map(|n| {
            let base = if n % 3 == 0 { DAY } else { NIGHT };
            theme(&format!("Sunrise variant {n}"), tweaked(n, base))
        })
        .collect();
    round_trips(ten);
}

#[test]
fn a_code_is_base64url_after_its_prefix_with_nothing_to_group_it() {
    let code = encode(&[theme("Ember", NIGHT), theme("Paper", DAY)]);
    assert_eq!(&code[..5], "SUN2.");
    assert!(
        code[5..]
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'),
        "{code}"
    );
    assert!(!code.contains(char::is_whitespace));
}

#[test]
fn how_a_code_is_pasted_does_not_matter() {
    let themes = vec![theme("Ember", NIGHT), theme("Paper", DAY)];
    let code = encode(&themes);
    // Wrapped over lines and padded with spaces, as a chat or an email does.
    let wrapped: String = code
        .as_bytes()
        .chunks(12)
        .map(|c| format!("  {}\r\n", std::str::from_utf8(c).unwrap()))
        .collect();
    assert_eq!(decode(&wrapped), Ok(themes.clone()));
    // The prefix may be in lower case; the rest is case sensitive.
    assert_eq!(decode(&format!("sun2.{}", &code[5..])), Ok(themes.clone()));
    // Base64's padding, which a tool may add back.
    assert_eq!(decode(&format!("{code}==")), Ok(themes));
}

#[test]
fn a_code_stays_short_as_themes_are_added() {
    // Two themes at Sunrise's palettes with short names.
    let two = encode(&[
        theme("Ember", Roles::DAWN),
        theme("Dusk", Roles::DAWN_LIGHT),
    ]);
    assert!(two.len() < 40, "{} {two}", two.len());
    // A copy of a theme costs its name and a header, about four bytes, or five or six
    // characters, more than the name itself.
    for name in ["Ember", "Sunrise over the quiet harbour"] {
        let one = encode(&[theme(name, NIGHT)]).len() as isize;
        let twice = encode(&[theme(name, NIGHT), theme(name, NIGHT)]).len() as isize;
        let allowed = ((name.len() + 4) * 4 / 3 + 1) as isize;
        assert!(twice - one <= allowed, "{name}: {one} then {twice}");
    }
    // Without compression: exactly a header, a length and the name.
    let one = body_of(&[theme("Ember", NIGHT)]).len();
    let twice = body_of(&[theme("Ember", NIGHT), theme("Ember", NIGHT)]).len();
    assert_eq!(twice - one, 2 + 1 + "Ember".len());
}

#[test]
fn a_theme_builds_on_the_one_before_only_when_that_saves_colours() {
    // A first theme of its own, a few colours away from Sunrise's.
    let own = Roles {
        line: 0x5a4a44,
        success: 0x5fc98a,
        danger: 0xe0706e,
        ..NIGHT
    };
    let changed = differences(&own, &Roles::DAWN).count_ones() as usize;
    assert!(changed >= 2, "the first theme must differ from Sunrise for this test");
    let body = body_of(&[
        theme("Ember", own),
        theme(
            "Ember Two",
            Roles {
                accent: 0x112233,
                ..own
            },
        ),
    ]);
    // Count, then each theme's header, length, name and changed colours.
    assert_eq!(body.len(), 1 + (3 + 5 + 3 * changed) + (3 + 9 + 3));
    // The second one's header points at the first.
    let second = 1 + 3 + 5 + 3 * changed;
    assert_eq!(body[second] & 0x04, 0x04);
    // A theme on the other look has nothing to build on but its own look's palette.
    let first = body_of(&[theme("Paper", DAY), theme("Ember", NIGHT)]);
    let mask = differences(&DAY, &Roles::DAWN_LIGHT).count_ones() as usize
        + differences(&NIGHT, &Roles::DAWN).count_ones() as usize;
    assert!(first.len() <= 1 + 3 + 5 + 3 + 5 + 3 * mask);
}

#[test]
fn many_themes_are_compressed_when_that_is_shorter() {
    let many: Vec<Custom> = (0..10)
        .map(|n| theme(&format!("Sunrise variant {n}"), tweaked(n, NIGHT)))
        .collect();
    let code = encode(&many);
    assert_ne!(bytes_of(&code)[0] & COMPRESSED, 0, "{code}");
    let raw = armour(false, &body_of(&many));
    assert!(
        code.len() < raw.len(),
        "{} against {}",
        code.len(),
        raw.len()
    );
    // One theme is not worth it.
    assert_eq!(bytes_of(&encode(&[theme("Ember", NIGHT)]))[0], VERSION);
}

#[test]
fn unicode_names_survive_and_long_ones_are_cut_between_characters() {
    for name in ["Café", "日本語の空", "🌅 Aurore", "Ünï"] {
        round_trips(vec![theme(name, NIGHT)]);
    }
    // 20 two-byte letters are 40 bytes: 16 of them fit.
    let long = "é".repeat(20);
    let read = decode(&encode(&[theme(&long, NIGHT)])).unwrap();
    assert_eq!(read[0].name, "é".repeat(16));
    // A cut that would land inside a character stops before it.
    let read = decode(&encode(&[theme(&format!("{}日", "a".repeat(31)), NIGHT)])).unwrap();
    assert_eq!(read[0].name, "a".repeat(31));
}

#[test]
fn a_theme_without_a_name_is_numbered() {
    let read = decode(&encode(&[
        theme("", NIGHT),
        theme("Ember", NIGHT),
        theme("", DAY),
    ]))
    .unwrap();
    assert_eq!(read[1].name, "Ember");
    for (n, at) in [(1, 0), (3, 2)] {
        assert!(read[at].name.contains(&n.to_string()), "{}", read[at].name);
    }
    // A name that is empty is the shortest there is: no bytes after its length.
    let body = body_of(&[theme("", Roles::DAWN)]);
    assert_eq!(body, vec![1, 0, 0, 0]);
}

#[test]
fn a_mistyped_character_is_a_checksum_error_not_other_colours() {
    let code = encode(&[theme("Ember", NIGHT)]);
    let mut chars: Vec<char> = code.chars().collect();
    let at = 20;
    chars[at] = if chars[at] == 'A' { 'B' } else { 'A' };
    let wrong: String = chars.into_iter().collect();
    assert_eq!(decode(&wrong), Err(CodeError::Checksum));
    // Case matters in a code: a lower-cased body doesn't add up.
    let lowered = format!("{PREFIX}{}", code[5..].to_lowercase());
    assert_eq!(decode(&lowered), Err(CodeError::Checksum));
}

#[test]
fn a_cut_off_code_is_refused() {
    let code = encode(&[theme("Ember", NIGHT), theme("Paper", DAY)]);
    for keep in [5, 6, 20, code.len() - 8, code.len() - 1] {
        assert!(decode(&code[..keep]).is_err(), "{keep}");
    }
    assert_eq!(decode(&code[..code.len() - 8]), Err(CodeError::Checksum));
    assert_eq!(decode(PREFIX), Err(CodeError::Checksum));
    // Bytes that stop in the middle of a theme but whose checksum is right.
    for body in [
        &[2u8, 0, 0][..],
        &[1, 0],
        &[1, 0, 0, 5, b'a'],
        &[1, 0, 32, 1, b'x', 1, 2],
    ] {
        assert_eq!(
            decode(&armour(false, body)),
            Err(CodeError::Truncated),
            "{body:?}"
        );
    }
}

#[test]
fn another_prefix_or_version_is_said_so() {
    let code = encode(&[theme("Ember", NIGHT)]);
    let body = &code[5..];
    assert_eq!(decode(&format!("ABC2.{body}")), Err(CodeError::Prefix));
    assert_eq!(decode(&format!("SUN{body}")), Err(CodeError::Prefix));
    assert_eq!(decode(&format!("SUN2{body}")), Err(CodeError::Prefix));
    assert_eq!(decode("hello"), Err(CodeError::Prefix));
    assert_eq!(decode(&format!("SUN3.{body}")), Err(CodeError::Version));
    assert_eq!(decode(&format!("sun9-{body}")), Err(CodeError::Version));
    // A layout of another version inside the right prefix, and a flag nobody knows.
    assert_eq!(decode(&code_of(3, &[1, 0, 0, 0])), Err(CodeError::Version));
    assert_eq!(
        decode(&code_of(VERSION | 0x20, &[1, 0, 0, 0])),
        Err(CodeError::Version)
    );
}

#[test]
fn nothing_and_nonsense_are_refused_with_their_own_reasons() {
    assert_eq!(decode(""), Err(CodeError::Empty));
    assert_eq!(decode(" \n "), Err(CodeError::Empty));
    assert_eq!(decode("SUN2.AB!D"), Err(CodeError::Characters('!')));
    assert_eq!(decode("SUN2.ABé"), Err(CodeError::Characters('é')));
    assert_eq!(decode(&armour(false, &[0])), Err(CodeError::Damaged));
    // Bytes left over after the last theme.
    assert_eq!(
        decode(&armour(false, &[1, 0, 0, 0, 7])),
        Err(CodeError::Damaged)
    );
    // Header bits that mean nothing yet, and a first theme that builds on one before it.
    assert_eq!(
        decode(&armour(false, &[1, 0x08, 0, 0])),
        Err(CodeError::Damaged)
    );
    assert_eq!(
        decode(&armour(false, &[1, 0x04, 0, 0])),
        Err(CodeError::Damaged)
    );
    // A name longer than a name can be, and one that isn't text.
    let mut long = vec![1, 0, 0, 33];
    long.extend([b'a'; 33]);
    assert_eq!(decode(&armour(false, &long)), Err(CodeError::Damaged));
    assert_eq!(
        decode(&armour(false, &[1, 0, 0, 1, 0xff])),
        Err(CodeError::Damaged)
    );
    // A body that claims to be compressed and isn't, or inflates to far too much.
    assert_eq!(decode(&armour(true, &[0xff; 5])), Err(CodeError::Damaged));
    let bomb = deflate(&vec![0u8; 1 << 20]).unwrap();
    assert_eq!(decode(&armour(true, &bomb)), Err(CodeError::Damaged));
    for e in [CodeError::Empty, CodeError::Prefix, CodeError::Checksum] {
        assert!(!e.text().is_empty());
    }
    assert!(CodeError::Prefix.text().contains("SUN2."));
}

#[test]
fn a_name_is_made_safe_and_text_that_cannot_be_read_is_refused() {
    let read = decode(&encode(&[theme("a\u{7}b\u{202e}c\nd", NIGHT)])).unwrap();
    assert_eq!(read[0].name, "abcd");
    let blank = decode(&encode(&[theme(" \u{7} ", NIGHT)])).unwrap();
    assert!(!blank[0].name.is_empty());
    let grey = Roles {
        background: 0x777777,
        card: 0x777777,
        chrome: 0x777777,
        ..NIGHT
    };
    assert_eq!(
        decode(&encode(&[theme("Mud", grey)])),
        Err(CodeError::Colours("Mud".into()))
    );
}

#[test]
fn a_code_holds_at_most_255_themes() {
    let many: Vec<Custom> = (0..300).map(|n| theme(&format!("T{n}"), NIGHT)).collect();
    let read = decode(&encode(&many)).unwrap();
    assert_eq!(read.len(), 255);
    assert_eq!(read[254].name, "T254");
}
