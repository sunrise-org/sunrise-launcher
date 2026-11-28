//! Discover's search: the sort, the filters, matching a query and ranking cards.
use super::*;

/// How Discover orders matching mods.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::mods) enum V2Sort {
    #[default]
    Relevance,
    Updated,
    Name,
}

impl V2Sort {
    pub(in crate::mods) const ALL: [V2Sort; 3] = [V2Sort::Relevance, V2Sort::Updated, V2Sort::Name];

    pub(in crate::mods) fn label(self) -> &'static str {
        match self {
            V2Sort::Relevance => tr("Relevance"),
            V2Sort::Updated => tr("Recently updated"),
            V2Sort::Name => tr("Name"),
        }
    }
}

/// Active Discover filters. `show_all` defaults to `false`: only mods with a fitting release for
/// the selected profile's exact build are shown.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(in crate::mods) struct V2Filters {
    pub(in crate::mods) text: String,
    pub(in crate::mods) category: Option<String>,
    pub(in crate::mods) show_all: bool,
    pub(in crate::mods) sort: V2Sort,
}

pub(super) fn query_words(text: &str) -> Vec<String> {
    text.split_whitespace().map(str::to_lowercase).collect()
}

pub(super) fn card_matches_query(card: &V2Card, words: &[String]) -> bool {
    words.iter().all(|w| {
        card.name_lc.contains(w)
            || card.slug_lc.contains(w)
            || card.mod_id_lc.contains(w)
            || card.summary_lc.contains(w)
            || card.authors_lc.contains(w)
            || card.repo_lc.contains(w)
            || card.categories_lc.iter().any(|c| c.contains(w))
            || card.description_lc.contains(w)
    })
}

fn relevance_score(card: &V2Card, full_query: &str, words: &[String]) -> u32 {
    if words.is_empty() {
        return 0;
    }
    let mut score: u32 = 0;
    if card.name_lc == full_query || card.mod_id_lc == full_query || card.slug_lc == full_query {
        score += 1000;
    } else if card.name_lc.starts_with(full_query) {
        score += 500;
    } else if card.name_lc.contains(full_query) {
        score += 250;
    }
    for word in words {
        if card
            .name_lc
            .split(|c: char| !c.is_alphanumeric())
            .any(|token| token == word)
        {
            score += 120;
        } else if card.name_lc.contains(word) {
            score += 80;
        }
        if card.mod_id_lc.contains(word) || card.slug_lc.contains(word) {
            score += 60;
        }
        if card.summary_lc.contains(word) {
            score += 30;
        }
        if card.authors_lc.contains(word) || card.repo_lc.contains(word) {
            score += 20;
        }
        if card.categories_lc.iter().any(|c| c.contains(word)) {
            score += 15;
        }
        if card.description_lc.contains(word) {
            score += 5;
        }
    }
    score
}

/// Indices into `cards` matching `filters`, ordered by `filters.sort`.
pub(in crate::mods) fn discover(cards: &[V2Card], filters: &V2Filters) -> Vec<usize> {
    let words = query_words(&filters.text);
    let full_query = filters.text.trim().to_lowercase();
    let mut scored: Vec<(usize, u32)> = cards
        .iter()
        .enumerate()
        .filter(|(_, c)| filters.show_all || c.fit.fits())
        .filter(|(_, c)| {
            filters
                .category
                .as_ref()
                .is_none_or(|k| c.categories.contains(k))
        })
        .filter(|(_, c)| card_matches_query(c, &words))
        .map(|(i, c)| (i, relevance_score(c, &full_query, &words)))
        .collect();

    scored.sort_by(|&(ai, ascore), &(bi, bscore)| {
        let (a, b) = (&cards[ai], &cards[bi]);
        match filters.sort {
            V2Sort::Relevance => bscore
                .cmp(&ascore)
                .then_with(|| {
                    if !words.is_empty() {
                        b.fit.fits().cmp(&a.fit.fits())
                    } else {
                        Ordering::Equal
                    }
                })
                .then_with(|| b.updated.cmp(&a.updated))
                .then_with(|| a.name_lc.cmp(&b.name_lc))
                .then_with(|| a.id.cmp(&b.id)),
            V2Sort::Updated => b
                .updated
                .cmp(&a.updated)
                .then_with(|| a.name_lc.cmp(&b.name_lc))
                .then_with(|| a.id.cmp(&b.id)),
            V2Sort::Name => a.name_lc.cmp(&b.name_lc).then_with(|| a.id.cmp(&b.id)),
        }
    });
    scored.into_iter().map(|(i, _)| i).collect()
}

/// How many cards match the active text and category filters but do not fit the selected build.
pub(in crate::mods) fn hidden_other_build_count(cards: &[V2Card], filters: &V2Filters) -> usize {
    let words = query_words(&filters.text);
    cards
        .iter()
        .filter(|c| !c.fit.fits())
        .filter(|c| {
            filters
                .category
                .as_ref()
                .is_none_or(|k| c.categories.contains(k))
        })
        .filter(|c| card_matches_query(c, &words))
        .count()
}
