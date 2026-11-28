//! The side column of a registry mod's page: plain fact rows, the links, the ratings, and at the
//! bottom the install block, each part set off from the next by a hairline.
use super::model::discover_v2::V2Page;
use super::model::SourceLink;
use super::*;
use gpui::Hsla;

/// A hairline between two parts of the column.
fn rule() -> Div {
    div().h(px(1.)).w_full().flex_none().bg(palette().rule())
}

/// One fact: a muted icon, its muted label, and the value (if it has one) on the right.
fn fact_row(icon: &'static str, tint: Hsla, label: &'static str, value: Option<Div>) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(12.))
        .text_size(px(tokens::LABEL))
        .child(crate::glass_ui::icon(icon, 18., tint))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_color(c(palette().muted))
                .child(label),
        )
        .children(value.map(|value| value.min_w_0().truncate()))
}

/// A link that opens `url`: an icon, the label, and the arrow that says it leaves the launcher.
fn link_row(
    id: &'static str,
    icon: &'static str,
    label: String,
    url: String,
    cx: &mut Context<Launcher>,
) -> Stateful<Div> {
    let p = palette();
    div()
        .id(id)
        .reach()
        .flex()
        .items_center()
        .gap(px(12.))
        .p(px(8.))
        .rounded(px(8.))
        .text_size(px(tokens::LABEL))
        .cursor_pointer()
        .hover(|s| s.bg(c(p.hover)))
        .child(crate::glass_ui::icon(icon, 18., c(p.muted)))
        .child(div().flex_1().min_w_0().truncate().child(label))
        .child(crate::glass_ui::icon("arrow-up-right-01", 14., c(p.muted)))
        .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
            Launcher::open_url(&url, cx);
        }))
}

fn link_url(link: &Option<SourceLink>) -> Option<String> {
    link.as_ref().map(|link| match link {
        SourceLink::GitHub(url) | SourceLink::Other(url) => url.clone(),
    })
}

/// "Today", "3 days ago", else the date; the text as it came when it isn't a date.
fn updated_ago(published: &str) -> String {
    use chrono::TimeZone;
    let when = chrono::DateTime::parse_from_rfc3339(published)
        .ok()
        .map(std::time::SystemTime::from)
        .or_else(|| {
            let day = chrono::NaiveDate::parse_from_str(published.get(..10)?, "%Y-%m-%d").ok()?;
            let noon = day.and_hms_opt(12, 0, 0)?;
            chrono::Local
                .from_local_datetime(&noon)
                .single()
                .map(std::time::SystemTime::from)
        });
    let Some(when) = when else {
        return published.to_owned();
    };
    let text = crate::worlds_page::ago(when);
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => text,
    }
}

impl Launcher {
    /// The side column. `install` is the bottom block ([`Launcher::install_block`]).
    pub(super) fn project_side(&self, vp: &V2Page, install: Div, cx: &mut Context<Self>) -> Div {
        let p = palette();
        let mut facts = div().flex().flex_col().gap(px(14.)).child(fact_row(
            "tag-01",
            c(p.muted),
            tr("In-game ID"),
            Some(
                div()
                    .font_family(crate::theme::MONO)
                    .child(vp.mod_id.clone()),
            ),
        ));
        if !vp.updated.is_empty() {
            facts = facts.child(fact_row(
                "clock",
                c(p.muted),
                tr("Updated"),
                Some(div().child(updated_ago(&vp.updated))),
            ));
        }
        if !vp.license.is_empty() {
            facts = facts.child(fact_row(
                "scale",
                c(p.muted),
                tr("Licence"),
                Some(div().child(vp.license.clone())),
            ));
        }
        facts = facts.child(fact_row(
            "shield-check",
            c(p.success),
            tr("Reviewed by Sunrise"),
            None,
        ));

        let source = link_url(&vp.source).or_else(|| link_url(&vp.repository_url));
        let mut links: Vec<Stateful<Div>> = Vec::new();
        if let Some(url) = source {
            let label = match vp.repository_name.is_empty() {
                true => tr("Source code").to_owned(),
                false => vp.repository_name.clone(),
            };
            links.push(link_row(
                "alt-v2-source-open",
                "source-code",
                label,
                url,
                cx,
            ));
        }
        if let Some(url) = link_url(&vp.homepage) {
            links.push(link_row(
                "alt-v2-homepage",
                "home-01",
                tr("Homepage").to_owned(),
                url,
                cx,
            ));
        }
        if let Some(url) = link_url(&vp.issues) {
            links.push(link_row(
                "alt-v2-issues",
                "alert-circle",
                tr("Report an issue").to_owned(),
                url,
                cx,
            ));
        }
        let has_links = !links.is_empty();

        div()
            .flex()
            .flex_col()
            .gap(px(16.))
            .child(facts)
            .child(rule())
            .when(has_links, |d| {
                d.child(div().flex().flex_col().gap(px(2.)).children(links))
                    .child(rule())
            })
            .child(self.rating_block(&vp.id, cx))
            .child(rule())
            .child(install)
    }

    /// The bottom block: the page's one action at full width, what it will install and whether it
    /// fits, then what the action did and the controls of an installed mod.
    pub(super) fn install_block(
        &self,
        vp: &V2Page,
        action: AnyElement,
        installed_id: Option<&str>,
        controls: Option<Div>,
        cx: &mut Context<Self>,
    ) -> Div {
        let p = palette();
        let fit = match &vp.will_install {
            Some(line) => Some(
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap(px(6.))
                    .text_size(px(tokens::CAPTION))
                    .text_color(c(p.success))
                    .child(crate::glass_ui::icon("tick-02", 14., c(p.success)))
                    .child(trf(
                        "{version} fits this build · {size}",
                        &[
                            ("version", &line.version),
                            ("size", &model::size(line.size)),
                        ],
                    )),
            ),
            None => vp.fit.reason().map(|reason| {
                div()
                    .text_size(px(tokens::CAPTION))
                    .text_center()
                    .text_color(c(p.warning))
                    .child(reason)
            }),
        };
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(action)
            .children(fit)
            .children(self.registry_card_outcome(&vp.id, cx))
            .children(installed_id.and_then(|id| self.registry_card_outcome(id, cx)))
            .children(controls)
    }
}
