use super::*;

#[test]
fn a_copy_shows_in_the_toast_only_while_it_fits_on_a_line() {
    assert_eq!(copied("AB12-CD\n"), Copied::Text("AB12-CD".into()));
    let long = "x".repeat(200);
    assert_eq!(
        copied(&long),
        Copied::Text(format!("{}…", "x".repeat(COPIED_SHOWN - 1)))
    );
    assert_eq!(
        copied("é".repeat(COPIED_SHOWN).as_str()),
        Copied::Text("é".repeat(COPIED_SHOWN))
    );
    assert_eq!(copied("error: one\n  two\n\nthree\n"), Copied::Lines(4));
    assert_eq!(copied("\nerror\n\n"), Copied::Lines(3));
}
