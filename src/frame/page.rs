//! The pages the rail leads to, and the state that times a switch between them.

use super::*;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum Page {
    #[default]
    Home,
    Discover,
    Installed,
    Packs,
    /// Packs › Discover (`Packs` is its Installed).
    PackFind,
    /// A pack's page, opened from Packs › Discover.
    PackProject,
    Project,
    Join,
    Hosting,
    /// The Skins tab (`skins_page`): the look of the account that plays.
    Skins,
    Settings,
}

impl Page {
    /// The pages `sunrise-launcher benchmark` switches through, as a player clicks them: the
    /// header's tabs, then Settings. Not Skins: opening it reads the account's profile (the
    /// keychain, Microsoft, Mojang) and keeps the skin worn, and a benchmark touches no account.
    pub(crate) const BENCH: [Page; 5] = [
        Page::Home,
        Page::Discover,
        Page::Packs,
        Page::Join,
        Page::Settings,
    ];

    /// The tab that stays selected on a subpage.
    pub(super) fn active_tab(self) -> Self {
        match self {
            Self::Installed | Self::Project => Self::Discover,
            Self::PackFind | Self::PackProject => Self::Packs,
            Self::Hosting => Self::Join,
            other => other,
        }
    }
}

#[derive(Default)]
pub(crate) struct State {
    pub page: Page,
    pub home: crate::home::HomeState,
    pub backdrop: crate::backdrop::Backdrop,
    pub sheet: Option<Sheet>,
    pub sheets: crate::sheets::SheetsState,
    pub settings: crate::settings::SettingsView,
    pub cat: crate::cat::CatState,
    /// The Logs panel: open or not, its tab and the lines it has read.
    pub logs: crate::logs_panel::LogsPanel,
    pub last_server: Option<Page>,
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
