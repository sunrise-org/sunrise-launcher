//! The pages the rail leads to, and the state that times a switch between them.

use super::*;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum Page {
    #[default]
    Home,
    Discover,
    Packs,
    /// Packs › Discover (`Packs` is the shelves of shader and data packs).
    PackFind,
    /// A pack's page, opened from Packs › Discover.
    PackProject,
    Project,
    Hosting,
    /// The Skins tab (`skins_page`): the look of the account that plays.
    Skins,
    Settings,
}

impl Page {
    /// The pages `sunrise-launcher benchmark` switches through, as a player clicks them: the
    /// header's tabs, then Settings. Not Skins: opening it reads the account's profile (the
    /// keychain, Microsoft, Mojang) and keeps the skin worn, and a benchmark touches no account.
    pub(crate) const BENCH: [Page; 4] = [Page::Home, Page::Discover, Page::Packs, Page::Settings];

    /// The tab that stays selected on a subpage.
    pub(crate) fn active_tab(self) -> Self {
        match self {
            Self::Project => Self::Discover,
            Self::PackFind | Self::PackProject => Self::Packs,
            Self::Hosting => Self::Home,
            other => other,
        }
    }
}

/// How many pages Back remembers.
const BACK_LIMIT: usize = 20;

/// The pages Back returns to, the most recent last, at most [`BACK_LIMIT`] of them. A page's own
/// state (Home's tab, its scroll, the profile) stays where it lives, so returning to the page is
/// returning to what the player left.
#[derive(Default)]
pub(crate) struct History(Vec<Page>);

impl History {
    /// Remembers `from` as the page Back returns to, dropping the oldest past the limit.
    pub(crate) fn remember(&mut self, from: Page) {
        self.0.push(from);
        if self.0.len() > BACK_LIMIT {
            self.0.remove(0);
        }
    }

    /// The page Back returns to, forgotten as it is taken.
    pub(crate) fn pop(&mut self) -> Option<Page> {
        self.0.pop()
    }

    /// A details page was left for `to` by its own way back: forget the entry that led to it, if
    /// it is still the latest, so Back doesn't return to a page the player is already on.
    pub(crate) fn leave_to(&mut self, to: Page) {
        if self.0.last() == Some(&to) {
            self.0.pop();
        }
    }

    #[cfg(test)]
    pub(crate) fn pages(&self) -> &[Page] {
        &self.0
    }
}

impl Page {
    /// Whether this page shows one mod's or pack's details.
    pub(crate) fn is_details(self) -> bool {
        matches!(self, Self::Project | Self::PackProject)
    }

    /// The page a mod's details opened from here return to: this one when it is a list the
    /// player can come back to, else Discover.
    pub(crate) fn details_origin(self) -> Self {
        match self {
            Self::Home | Self::Discover | Self::Packs | Self::PackFind => self,
            _ => Self::Discover,
        }
    }
}

#[derive(Default)]
pub(crate) struct State {
    pub page: Page,
    pub back: History,
    pub home: crate::home::HomeState,
    pub backdrop: crate::backdrop::Backdrop,
    pub sheet: Option<Sheet>,
    pub sheets: crate::sheets::SheetsState,
    pub settings: crate::settings::SettingsView,
    pub cat: crate::cat::CatState,
    pub after_install: Option<(crate::launcher_play::PlayIntent, String)>,
    /// A Play (including a destination) that explicitly asked for the sign-in now under way.
    pub after_sign_in: Option<crate::launcher_play::PlayIntent>,
    /// The sidebar's last open or close, while it may still be easing there.
    pub rail: Option<crate::motion::Fade>,
    /// How open the sidebar is in the frame being drawn, or was in the last one: sampled once
    /// as each frame begins, so the rail, the top bar and the page lay out for the same width.
    pub rail_drawn: Option<f32>,
    /// Tests: draw the sidebar this far open, as if part way through an ease.
    #[cfg(test)]
    pub rail_pin: Option<f32>,
    /// The sidebar's Help & community section is open (its Discord and Website rows show).
    pub help_open: bool,
    /// Where the last frame drew each profile that has a context menu: the open menu covers
    /// them, so a right-click on one reaches it through here.
    pub profile_rows: crate::rail::ProfileRows,
    /// When the status strip appeared, so each appearance eases in once without jumping the page.
    pub status_strip_since: Option<Instant>,
    /// When the status strip went away, so its space closes as smoothly as it opened.
    pub status_strip_gone: Option<Instant>,
    /// The last drawn page and when we switched to it (None on first draw or once settled).
    pub page_switched: Option<(Page, Option<Instant>)>,
}

impl State {
    /// Remembers `from` as the page Back returns to.
    pub(crate) fn remember(&mut self, from: Page) {
        self.back.remember(from);
    }

    /// A details page is opening from the page shown: Back returns there. Moving between
    /// details pages (a dependency, a collection's mod) adds nothing: they share one way back.
    pub(crate) fn enter_details(&mut self) {
        if !self.page.is_details() {
            let from = self.page;
            self.back.remember(from);
        }
    }

    /// Returns the instant the current page entrance started while its transition is still active,
    /// or `None` on initial render, under reduced motion, or after [`crate::motion::PAGE`] ends.
    pub(crate) fn page_transition(
        &mut self,
        page: Page,
        now: Instant,
        still: bool,
    ) -> Option<Instant> {
        match self.page_switched {
            Some((last_page, switched_at)) => {
                if last_page.active_tab() != page.active_tab() {
                    let started = (!still).then_some(now);
                    self.page_switched = Some((page, started));
                    started
                } else {
                    if last_page != page {
                        self.page_switched = Some((page, switched_at));
                    }
                    if still {
                        None
                    } else {
                        switched_at.filter(|&at| {
                            now.saturating_duration_since(at) < crate::motion::PAGE.duration()
                        })
                    }
                }
            }
            None => {
                self.page_switched = Some((page, None));
                None
            }
        }
    }
}

#[cfg(test)]
mod tests;
