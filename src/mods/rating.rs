//! A registry mod's ratings on its page: the average and count everyone sees, and five stars the
//! selected account rates with (`browser::sunrise::ratings`). The totals are read once a run, when
//! the first mod page opens; the account's own rating once per mod and account.
use super::ui::{c, muted, note};
use super::*;
use crate::paint::palette;
use sunrise_launcher::browser::sunrise::ratings::{self, Totals};

#[derive(Default)]
pub(super) struct Ratings {
    /// Every rated mod's totals; none until the registry answered.
    totals: Option<HashMap<String, Totals>>,
    /// The registry didn't answer for the totals: the page shows none this run.
    totals_failed: bool,
    asked: bool,
    /// Whose own ratings `mine` holds.
    account: Option<String>,
    /// The account's own rating per mod; a mod is here once it was asked for.
    mine: HashMap<String, Option<u8>>,
    /// The mod whose rating is being sent.
    sending: Option<String>,
    /// Why the last rating of a mod wasn't taken.
    error: Option<(String, String)>,
}

pub(super) enum Answer {
    Totals(Result<HashMap<String, Totals>, String>),
    /// The account's own rating of a mod.
    Mine(String, String, Option<u8>),
    /// A rating was sent: by whom, for which mod, how many stars, and the mod's new totals.
    Rated(String, String, u8, Result<Totals, String>),
}

impl Launcher {
    /// Asks for what a mod page shows that isn't held yet: the totals, and the selected account's
    /// own rating of `id`.
    pub(super) fn ratings_for_page(&mut self, id: &str) {
        let account = self.accounts.selected().map(|a| a.uuid.clone());
        let r = &mut self.mods_state.ratings;
        if !r.asked {
            r.asked = true;
            let tx = self.mods_state.tx.clone();
            std::thread::spawn(move || {
                let _ = tx.send(Event::Rating(Answer::Totals(ratings::totals())));
            });
        }
        if r.account != account {
            r.account = account.clone();
            r.mine.clear();
            r.error = None;
        }
        let Some(account) = account else {
            return;
        };
        if r.mine.contains_key(id) {
            return;
        }
        r.mine.insert(id.to_owned(), None);
        let (tx, id) = (self.mods_state.tx.clone(), id.to_owned());
        std::thread::spawn(move || {
            // Not knowing it only leaves the stars empty: rating again replaces it either way.
            let stars = ratings::mine(&account, &id).ok().flatten();
            let _ = tx.send(Event::Rating(Answer::Mine(account, id, stars)));
        });
    }

    fn rate(&mut self, id: String, stars: u8) {
        let Some(account) = self.accounts.selected().map(|a| a.uuid.clone()) else {
            return;
        };
        let r = &mut self.mods_state.ratings;
        if r.sending.is_some() {
            return;
        }
        r.sending = Some(id.clone());
        r.error = None;
        let tx = self.mods_state.tx.clone();
        std::thread::spawn(move || {
            let result = ratings::rate(&account, &id, stars);
            let _ = tx.send(Event::Rating(Answer::Rated(account, id, stars, result)));
        });
    }

    pub(super) fn rating_answer(&mut self, answer: Answer) {
        let r = &mut self.mods_state.ratings;
        match answer {
            // Without totals the page shows no average; it never says the registry is down.
            Answer::Totals(Ok(totals)) => r.totals = Some(totals),
            Answer::Totals(Err(_)) => r.totals_failed = true,
            Answer::Mine(account, id, stars) => {
                if r.account.as_ref() == Some(&account) {
                    r.mine.insert(id, stars);
                }
            }
            Answer::Rated(account, id, stars, result) => {
                r.sending = None;
                if r.account.as_ref() != Some(&account) {
                    return;
                }
                match result {
                    Ok(totals) => {
                        r.mine.insert(id.clone(), Some(stars));
                        if let Some(held) = r.totals.as_mut() {
                            held.insert(id, totals);
                        }
                    }
                    Err(why) => r.error = Some((id, why)),
                }
            }
        }
    }

    /// The ratings block of mod `id`'s page: its title with the count beside it, then the stars.
    pub(super) fn rating_block(&self, id: &str, cx: &mut Context<Self>) -> Div {
        let p = palette();
        let r = &self.mods_state.ratings;
        let beside = match r.totals.as_ref().map(|t| t.get(id)) {
            None if r.totals_failed => None,
            None => Some(tr("Loading ratings…").to_owned()),
            Some(Some(t)) if t.count > 0 => Some(if t.count == 1 {
                format!("{:.1} · {}", t.average, tr("1 rating"))
            } else {
                format!(
                    "{:.1} · {}",
                    t.average,
                    trf("{count} ratings", &[("count", &t.count.to_string())])
                )
            }),
            Some(_) => Some(tr("none yet").to_owned()),
        };
        let heading = div()
            .flex()
            .items_baseline()
            .gap(px(8.))
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_size(px(tokens::LABEL))
                    .child(tr("Ratings")),
            )
            .children(beside.map(muted));
        let mut block = div().flex().flex_col().gap(px(8.)).child(heading);
        let account = self.accounts.selected().map(|a| &a.uuid);
        if account.is_none() {
            return block.child(muted(tr("Sign in to rate this mod.")));
        }
        let mine = match r.account.as_ref() == account {
            true => r.mine.get(id).copied().flatten().unwrap_or(0),
            false => 0,
        };
        let sending = r.sending.as_deref() == Some(id);
        let mut stars = div().flex().items_center().gap(px(4.));
        for n in 1..=5u8 {
            let lit = n <= mine;
            let star = div()
                .id(SharedString::from(format!("rating-star-{n}")))
                .p(px(2.))
                .rounded(crate::tokens::r(crate::tokens::R_SMALL))
                .child(crate::glass_ui::icon(
                    if lit { "star-filled" } else { "star" },
                    28.,
                    c(if lit { p.accent } else { p.muted }),
                ));
            stars = stars.child(if sending {
                star.opacity(0.5)
            } else {
                let id = id.to_owned();
                star.cursor_pointer()
                    .hover(|s| s.bg(c(p.field)))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.rate(id.clone(), n);
                        cx.notify();
                    }))
            });
        }
        block = block.child(stars).child(muted(if sending {
            tr("Sending…")
        } else if mine > 0 {
            tr("Your rating")
        } else {
            tr("Rate this mod")
        }));
        match &r.error {
            Some((mod_id, why)) if mod_id == id => block.child(note(why.clone())),
            _ => block,
        }
    }
}
