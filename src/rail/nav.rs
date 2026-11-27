//! The sidebar's navigation: Discover, Servers and Skins at the top, Help & community (with its
//! Discord and Website rows) and Settings at the foot. Each row is the shared `nav_row`.
use super::*;
use crate::widgets::nav::nav_row;
use gpui::{AnimationExt, AnyElement};

/// The gap from a row's icon to its label: the label starts where a profile's name does.
const LABEL_GAP: f32 = 13.;
/// The height of the Help & community rows when open: two 36 px rows.
const HELP_ROWS: f32 = 72.;

/// One nav row, as the sidebar draws it at `open`. Its label and sun show once the sidebar is
/// open, and it is an icon alone before that (a closed sidebar's row is centred on its icon,
/// with the label's tooltip). The caller adds the click handler.
fn nav_item(
    open: f32,
    id: &'static str,
    icon: &'static str,
    label: &str,
    selected: bool,
    indent: bool,
) -> Stateful<Div> {
    let labelled = open >= 1.;
    let shown = if labelled { label } else { "" };
    let row = nav_row(id, icon, shown, selected)
        .px(px(ICON_PAD))
        .gap(px(LABEL_GAP));
    if labelled {
        row.when(indent, |d| d.pl(px(ICON_PAD + 14.)))
    } else {
        row.tooltip(crate::glass_ui::tip(label.to_owned()))
    }
}

/// A Discord or Website row under Help & community.
fn help_link(
    open: f32,
    id: &'static str,
    icon: &'static str,
    label: &str,
    url: String,
    cx: &mut Context<Launcher>,
) -> Stateful<Div> {
    nav_item(open, id, icon, label, false, true).when(!url.is_empty(), |d| {
        d.on_click(cx.listener(move |_, _, _, cx| {
            Launcher::open_url(&url, cx);
            cx.notify();
        }))
    })
}

impl Launcher {
    /// Discover, Servers and Skins. Discover is selected on the Mods and Packs pages (and their
    /// project pages); Servers goes back to the server page last open, as the header's tab did.
    pub(super) fn nav_section(&mut self, open: f32, cx: &mut Context<Self>) -> Div {
        let active = self.ui.page.active_tab();
        let discover = nav_item(
            open,
            "nav-discover",
            "compass-01",
            tr("Discover mods & packs"),
            matches!(active, Page::Discover | Page::Packs),
            false,
        )
        .on_click(cx.listener(|this, _, _, cx| {
            this.navigate(Page::Discover);
            cx.notify();
        }));
        let servers = nav_item(
            open,
            "nav-servers",
            "server-stack",
            tr("Servers"),
            active == Page::Join,
            false,
        )
        .on_click(cx.listener(|this, _, _, cx| {
            let to = this.ui.last_server.unwrap_or(Page::Join);
            this.navigate(to);
            cx.notify();
        }));
        let skins = nav_item(
            open,
            "nav-skins",
            "t-shirt",
            tr("Skins"),
            active == Page::Skins,
            false,
        )
        .on_click(cx.listener(|this, _, _, cx| {
            this.navigate(Page::Skins);
            cx.notify();
        }));
        div()
            .flex()
            .flex_col()
            .flex_none()
            .gap(px(4.))
            // The toggle's centre is 24 px down the 48 px bar; this row's is 40 px under it.
            .mt(px(-2.))
            .child(discover)
            .child(servers)
            .child(skins)
    }

    /// Help & community: a divider, the row that opens its two links, and the links while it is
    /// open. Opening and closing eases the links' height (instantly under reduced motion).
    pub(super) fn help_section(&mut self, open: f32, cx: &mut Context<Self>) -> Div {
        let p = palette();
        let help_open = self.ui.help_open;
        let mut help = nav_item(
            open,
            "sidebar-help",
            "help-circle",
            tr("Help & community"),
            false,
            false,
        )
        .on_click(cx.listener(|this, _, _, cx| {
            this.ui.help_open = !this.ui.help_open;
            cx.notify();
        }));
        if open >= 1. {
            help = help.child(crate::widgets::motion_bits::disclosure(
                "sidebar-help-chevron",
                help_open,
                12.,
                rgb(p.chrome_muted).into(),
            ));
        }
        div()
            .flex()
            .flex_col()
            .flex_none()
            .child(divider(&p))
            .child(help)
            .child(self.help_rows(open, help_open, cx))
    }

    /// The Discord and Website rows, at the height the open state is easing to. They are left
    /// out of the tree once closed, so they are not Tab stops while hidden.
    fn help_rows(&mut self, open: f32, help_open: bool, cx: &mut Context<Self>) -> AnyElement {
        let slide = crate::motion::sliding("sidebar-help-rows", help_open);
        let mut rows = div()
            .flex()
            .flex_col()
            .flex_none()
            .overflow_hidden()
            .h(px(if help_open { HELP_ROWS } else { 0. }));
        if help_open || slide.is_some() {
            rows = rows
                .child(help_link(
                    open,
                    "sidebar-discord",
                    "discord",
                    tr("Discord"),
                    self.links.discord.clone(),
                    cx,
                ))
                .child(help_link(
                    open,
                    "sidebar-website",
                    "earth",
                    tr("Website"),
                    self.links.website.clone(),
                    cx,
                ));
        }
        match slide {
            Some(n) => rows
                .with_animation(
                    ElementId::named_usize("sidebar-help-rows", n),
                    crate::motion::slide_motion(),
                    move |d, v| {
                        let v = if help_open { v } else { 1. - v };
                        d.h(px(HELP_ROWS * v))
                    },
                )
                .into_any_element(),
            None => rows.into_any_element(),
        }
    }

    /// Settings, at the foot, selected on the Settings page. It sits 4 px below the Help rows,
    /// as the nav rows sit 4 px apart.
    pub(super) fn settings_row(&mut self, open: f32, cx: &mut Context<Self>) -> Stateful<Div> {
        let selected = self.ui.page == Page::Settings;
        nav_item(
            open,
            "sidebar-settings",
            "settings-01",
            tr("Settings"),
            selected,
            false,
        )
        .mt(px(4.))
        .on_click(cx.listener(|this, _, _, cx| {
            this.navigate(Page::Settings);
            cx.notify();
        }))
    }
}
