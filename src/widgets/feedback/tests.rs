use super::*;

#[test]
fn a_bar_never_drops_below_a_sliver_and_never_passes_full() {
    assert_eq!(clamp_fraction(0.), 0.02);
    assert_eq!(clamp_fraction(0.5), 0.5);
    assert_eq!(clamp_fraction(3.), 1.);
}

#[test]
fn an_empty_state_line_loses_its_full_stop_only() {
    assert_eq!(
        label_line("Nothing here yet.".into()).as_ref(),
        "Nothing here yet"
    );
    assert_eq!(label_line("Nothing here。".into()).as_ref(), "Nothing here");
    assert_eq!(label_line("Add one.  ".into()).as_ref(), "Add one.  ");
}

#[test]
fn the_home_folder_is_written_as_a_tilde() {
    assert_eq!(
        tilde_with("/home/me/.config/s.json: full", Some("/home/me")),
        "~/.config/s.json: full"
    );
    assert_eq!(tilde_with("/home/meg/x", Some("/home/me")), "/home/meg/x");
    assert_eq!(tilde_with("/home/me/x", None), "/home/me/x");
    assert_eq!(tilde_with("/x", Some("/")), "/x");
}

#[test]
fn a_system_error_is_cut_to_its_reason() {
    assert_eq!(
        os_reason("/tmp/settings.json: Disk quota exceeded (os error 122)").as_deref(),
        Some("Disk quota exceeded")
    );
    assert_eq!(os_reason("no reason here"), None);
}

#[test]
fn a_title_of_several_lines_is_a_title_and_a_detail() {
    let (title, detail) = split_title("Failed\nBecause of this".into(), None);
    assert_eq!(title.as_ref(), "Failed");
    assert_eq!(detail.as_deref(), Some("Because of this"));
    let (title, detail) = split_title("One line".into(), None);
    assert_eq!((title.as_ref(), detail.as_deref()), ("One line", None));
    let (title, detail) = split_title("A\nB".into(), Some("given".into()));
    assert_eq!((title.as_ref(), detail.as_deref()), ("A\nB", Some("given")));
}
