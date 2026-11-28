use super::*;

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

#[test]
fn one_theme_is_the_code_it_always_was() {
    // The format is a promise to everyone holding a code: this one must keep reading.
    let code = "SUN1-040GA-HBDC9-JQ401-B4CJ1-W6RT6-8M2KW-ZAWTW-TQ9P2-AMPZ7-TQ6FQ-4RSV4-DHN5S-E";
    assert_eq!(encode(&[theme("Ember", NIGHT)]), code);
    assert_eq!(decode(code), Ok(vec![theme("Ember", NIGHT)]));
}

#[test]
fn several_themes_share_one_code() {
    let all = vec![
        theme("Ember", NIGHT),
        theme("Paper", DAY),
        theme(
            "Ember Two",
            Roles {
                accent: 0x112233,
                ..NIGHT
            },
        ),
    ];
    let code = encode(&all);
    assert_eq!(decode(&code), Ok(all));
}

#[test]
fn a_code_is_groups_of_five_after_its_prefix() {
    let code = encode(&[theme("Ember", NIGHT), theme("Paper", DAY)]);
    let (head, body) = code.split_once('-').unwrap();
    assert_eq!(head, "SUN1");
    let groups: Vec<&str> = body.split('-').collect();
    let (last, whole) = groups.split_last().unwrap();
    assert!(whole.iter().all(|g| g.len() == 5), "{code}");
    assert!((1..=5).contains(&last.len()), "{code}");
    assert!(code[5..]
        .chars()
        .all(|c| c == '-' || ALPHABET.contains(&(c as u8))));
}

#[test]
fn unicode_names_survive_and_long_ones_are_cut_between_characters() {
    for name in ["Café", "日本語の空", "🌅 Aurore", "Ünï"] {
        assert_eq!(
            decode(&encode(&[theme(name, NIGHT)])),
            Ok(vec![theme(name, NIGHT)])
        );
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
fn how_a_code_is_typed_does_not_matter() {
    let themes = vec![theme("Ember", NIGHT), theme("Paper", DAY)];
    let code = encode(&themes);
    let spaced = code.to_lowercase().replace('-', " \n");
    assert_eq!(decode(&spaced), Ok(themes.clone()));
    assert_eq!(decode(&code.replace('-', "")), Ok(themes.clone()));
    // Hyphens anywhere after the prefix, or none.
    let (head, body) = code.split_once('-').unwrap();
    assert_eq!(
        decode(&format!("{head}-{}", body.replace('-', ""))),
        Ok(themes.clone())
    );
    // O reads as 0 and I and L as 1.
    let misread = code.replace('0', "O").replace('1', "l");
    assert_eq!(decode(&misread.replace("SUNl", "SUN1")), Ok(themes));
}

#[test]
fn a_mistyped_character_is_a_checksum_error_not_other_colours() {
    let code = encode(&[theme("Ember", NIGHT)]);
    let mut chars: Vec<char> = code.chars().collect();
    let at = 20;
    chars[at] = if chars[at] == 'A' { 'B' } else { 'A' };
    let wrong: String = chars.into_iter().collect();
    assert_eq!(decode(&wrong), Err(CodeError::Checksum));
}

#[test]
fn a_cut_off_code_is_refused() {
    let code = encode(&[theme("Ember", NIGHT), theme("Paper", DAY)]);
    for keep in [6, 20, code.len() - 8, code.len() - 1] {
        assert!(decode(&code[..keep]).is_err(), "{keep}");
    }
    assert_eq!(decode(&code[..code.len() - 8]), Err(CodeError::Checksum));
    // Bytes that stop in the middle of a theme but whose checksum is right.
    assert_eq!(
        decode(&armour(vec![VERSION, 2, 3])),
        Err(CodeError::Truncated)
    );
    assert_eq!(
        decode(&armour(vec![VERSION, 1, 0, 0, 1, 2])),
        Err(CodeError::Truncated)
    );
}

#[test]
fn another_prefix_or_version_is_said_so() {
    let code = encode(&[theme("Ember", NIGHT)]);
    let body = &code[4..];
    assert_eq!(decode(&format!("ABC1{body}")), Err(CodeError::Prefix));
    assert_eq!(decode(&format!("SUN{body}")), Err(CodeError::Prefix));
    assert_eq!(decode("hello"), Err(CodeError::Prefix));
    assert_eq!(decode(&format!("SUN2{body}")), Err(CodeError::Version));
    assert_eq!(decode(&format!("sun9{body}")), Err(CodeError::Version));
    // A layout of another version inside the right prefix.
    assert_eq!(decode(&armour(vec![2, 0])), Err(CodeError::Version));
}

#[test]
fn nothing_and_nonsense_are_refused_with_their_own_reasons() {
    assert_eq!(decode(""), Err(CodeError::Empty));
    assert_eq!(decode(" \n "), Err(CodeError::Empty));
    assert_eq!(decode("SUN1-ABCDE-U!"), Err(CodeError::Characters('U')));
    assert_eq!(decode("SUN1-"), Err(CodeError::Checksum));
    assert_eq!(decode(&armour(vec![VERSION, 0])), Err(CodeError::Damaged));
    // Bytes left over after the last theme.
    let mut bytes = vec![VERSION, 1, 1, b'x', 0];
    bytes.extend([0x10; 27]);
    bytes.push(7);
    assert_eq!(decode(&armour(bytes)), Err(CodeError::Damaged));
    // A look byte that is neither.
    let mut bytes = vec![VERSION, 1, 1, b'x', 9];
    bytes.extend([0x10; 27]);
    assert_eq!(decode(&armour(bytes)), Err(CodeError::Damaged));
    for e in [CodeError::Empty, CodeError::Prefix, CodeError::Checksum] {
        assert!(!e.text().is_empty());
    }
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
