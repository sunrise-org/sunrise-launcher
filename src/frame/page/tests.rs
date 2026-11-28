use super::*;

#[test]
fn details_opened_from_home_go_back_to_home() {
    let mut history = History::default();
    history.remember(Page::Home);
    history.leave_to(Page::Home);
    assert!(history.pages().is_empty());
}

#[test]
fn leaving_to_a_page_that_is_not_the_latest_keeps_the_history() {
    let mut history = History::default();
    history.remember(Page::Discover);
    history.remember(Page::Settings);
    history.leave_to(Page::Discover);
    assert_eq!(history.pages(), [Page::Discover, Page::Settings]);
}

#[test]
fn history_keeps_only_the_latest_pages() {
    let mut history = History::default();
    for _ in 0..BACK_LIMIT + 5 {
        history.remember(Page::Discover);
    }
    history.remember(Page::Home);
    assert_eq!(history.pages().len(), BACK_LIMIT);
    assert_eq!(history.pop(), Some(Page::Home));
}

#[test]
fn only_lists_are_an_origin_for_details() {
    assert_eq!(Page::Home.details_origin(), Page::Home);
    assert_eq!(Page::Packs.details_origin(), Page::Packs);
    assert_eq!(Page::Settings.details_origin(), Page::Discover);
    assert!(Page::Project.is_details() && Page::PackProject.is_details());
    assert!(!Page::Home.is_details());
}
