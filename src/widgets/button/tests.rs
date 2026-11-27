use super::*;

#[test]
fn look_builders_set_one_field_each() {
    let look = Look::new(Style::Accent, Size::Small);
    assert_eq!(look.style, Style::Accent);
    assert_eq!(look.size, Size::Small);
    assert_eq!(look.icon, None);
    assert!(!look.disabled);

    let look = look.icon("plus-sign").disabled(true);
    assert_eq!(look.icon, Some("plus-sign"));
    assert!(look.disabled);
    assert_eq!(look.style, Style::Accent);
}

#[test]
fn medium_is_the_control_height_and_small_is_shorter() {
    assert_eq!(Size::Medium.height(), tokens::CONTROL);
    assert_eq!(Size::Small.height(), 28.);
    assert!(Size::Small.height() < Size::Medium.height());
    assert!(Size::Small.pad() < Size::Medium.pad());
    assert!(Size::Small.gap() < Size::Medium.gap());
}

#[test]
fn primary_label_is_on_the_accent_and_outlines_keep_their_edges() {
    assert_eq!(ink(Style::Primary), palette().accent_on);
    assert_eq!(ink(Style::Ghost), palette().text);
    assert_eq!(edge(Style::Secondary), Some(palette().line));
    assert_eq!(edge(Style::Accent), Some(palette().accent_ink));
    assert_eq!(edge(Style::Primary), None);
    assert_eq!(edge(Style::Ghost), None);
}
