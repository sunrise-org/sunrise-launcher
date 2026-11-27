//! Hosting's "Friends over the internet" in the new look: Tailscale on this computer, the address
//! friends type, inviting one of them to this computer, and keeping shared friends to the
//! server's port. Every action is `internet.rs`'s own; only the drawing is new.
use super::*;
use crate::paint::palette;
use crate::settings::{danger, hint, off};
use crate::widgets::button::secondary;
use crate::widgets::feedback::{banner, Tone};
use gpui::rgb;

fn link(id: &'static str, name: &'static str, url: &'static str) -> Stateful<Div> {
    secondary(id, tr(name)).on_click(move |_, _, cx| cx.open_url(url))
}

impl Launcher {
    fn share_button(
        &self,
        id: impl Into<ElementId>,
        text: impl Into<SharedString>,
        enabled: bool,
        cx: &mut Context<Self>,
        action: impl Fn(&mut Self, &mut Context<Self>) + 'static,
    ) -> Stateful<Div> {
        let b = secondary(id, text);
        if enabled {
            b.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                action(this, cx);
                cx.notify();
            }))
        } else {
            off(b)
        }
    }

    fn share_copy(&self, id: &'static str, text: String, cx: &mut Context<Self>) -> Stateful<Div> {
        let copied = self
            .internet
            .copied
            .as_ref()
            .is_some_and(|(what, at)| what == &text && at.elapsed() < Duration::from_secs(2));
        self.share_button(
            id,
            tr(if copied { "Copied" } else { "Copy" }),
            true,
            cx,
            move |this, cx| {
                crate::selectable::clipboard::copy_to_clipboard(
                    cx,
                    gpui::ClipboardItem::new_string(text.clone()),
                );
                this.internet.copied = Some((text.clone(), Instant::now()));
            },
        )
    }

    fn share_field(
        &mut self,
        f: Field,
        placeholder: &'static str,
        width: f32,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let focused = self.servers.has_focus(f);
        self.field(f, placeholder, width, cx)
            .h(px(tokens::FIELD))
            .px(px(12.))
            .rounded(tokens::r(tokens::R_CONTROL))
            .border_1()
            .border_color(if focused {
                rgb(palette().focus).into()
            } else {
                palette().input_edge()
            })
            .bg(rgb(palette().field))
            .text_size(px(13.))
            .text_color(rgb(palette().text))
    }

    pub(crate) fn hosting_share(
        &mut self,
        listener: Option<std::net::SocketAddr>,
        cx: &mut Context<Self>,
    ) -> Div {
        let mut panel = div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .p(px(16.))
            .rounded(tokens::r(tokens::R_CARD))
            .border_1()
            .border_color(palette().rule())
            .bg(palette().card())
            .child(
                div()
                    .text_size(px(11.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(palette().muted))
                    .child(tr("Friends over the internet · Tailscale").to_uppercase()),
            );
        let check = self.share_button("share-check", tr("Check again"), true, cx, |this, _| {
            this.internet_refresh()
        });
        let actions = |first: Stateful<Div>, second: Stateful<Div>| {
            div().flex().gap(px(6.)).child(first).child(second)
        };
        match self.internet.state.clone() {
            None => return panel.child(hint(tr("Looking for Tailscale…"))),
            Some(State::Missing) => {
                return panel
                    .child(crate::selectable::selectable(tr(
                        "Tailscale lets friends anywhere join without opening your router to the internet. It's free: install it, sign in, then invite them here.",
                    )))
                    .child(actions(
                        link("share-install", "Install Tailscale", tailscale::install_url()),
                        check,
                    ));
            }
            Some(State::Off(why)) => {
                let open =
                    self.share_button("share-open", tr("Open Tailscale"), true, cx, |this, _| {
                        if let Err(e) = tailscale::open_app() {
                            this.internet.message = Some(Err(e));
                        }
                        this.internet.read = None;
                    });
                panel = panel
                    .child(crate::selectable::selectable(tr(
                        "Tailscale is installed but not running. Open it and switch it on.",
                    )))
                    .children((!why.is_empty()).then(|| hint(why)))
                    .child(actions(open, check));
                return self.share_message(panel);
            }
            Some(State::SignedOut { auth_url }) => {
                let sign_in = self.share_button(
                    "share-login",
                    tr("Sign in to Tailscale"),
                    true,
                    cx,
                    move |this, cx| {
                        match &auth_url {
                            Some(url) => cx.open_url(url),
                            None => {
                                if let Err(e) = tailscale::open_app() {
                                    this.internet.message = Some(Err(e));
                                }
                            }
                        }
                        this.internet.read = None;
                    },
                );
                panel = panel
                    .child(crate::selectable::selectable(tr(
                        "Tailscale is installed. Sign in with your own account in Tailscale (Sunrise never sees it), then come back here.",
                    )))
                    .children(cfg!(target_os = "linux").then(|| {
                        hint(trf(
                            "On Linux, run `{cmd}` in a terminal and open the link it prints.",
                            &[("cmd", tailscale::LINUX_LOGIN)],
                        ))
                    }))
                    .child(actions(sign_in, check));
                return self.share_message(panel);
            }
            Some(State::Running(_)) => {}
        }
        let node = self.internet.node().cloned().unwrap_or_default();
        let ip = node
            .ipv4()
            .and_then(|ip| ip.parse::<std::net::IpAddr>().ok());
        let port = listener
            .filter(|address| {
                address.is_ipv4()
                    && !address.ip().is_loopback()
                    && ip.is_some_and(|ip| address.ip().is_unspecified() || address.ip() == ip)
            })
            .map(|address| address.port());
        let ip = ip.map(|ip| ip.to_string()).unwrap_or_default();
        if let Some(port) = port {
            let address = node.address(port);
            let typed = tr("Friends type it in Multiplayer, Add Server, once this computer is shared with them and Tailscale runs on theirs.");
            panel = panel
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(12.))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .font_family(theme::MONO)
                                .text_size(px(18.))
                                // A long name breaks after a dot, never inside a word: a zero-width
                                // space after each dot is a place it may wrap. Copy takes the plain one.
                                .child(address.replace('.', ".\u{200B}")),
                        )
                        .child(self.share_copy("share-copy", address, cx)),
                )
                .child(hint(if node.magic_dns && !ip.is_empty() {
                    let ip_port = if port == 25565 {
                        ip.clone()
                    } else {
                        format!("{ip}:{port}")
                    };
                    trf(
                        "Or {address}. {typed}",
                        &[("address", &ip_port), ("typed", typed)],
                    )
                } else {
                    typed.to_string()
                }));
        } else {
            self.internet.confirm_limit = false;
            panel = panel.child(hint(tr("No game address is available through Tailscale.")));
        }
        panel = panel
            .child(div().h(px(1.)).bg(palette().rule()))
            .child(
                div()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(tr("Invite a friend")),
            )
            .child(self.share_invite(&ip, port, cx))
            .child(hint(tr(
                "Sunrise never turns on Funnel: the server isn't put on the public internet, only shared with the people you invite.",
            )));
        self.share_message(panel)
    }

    fn share_message(&self, panel: Div) -> Div {
        match &self.internet.message {
            Some(Ok(m)) => panel.child(hint(m.clone())),
            Some(Err(e)) => panel.child(banner(Tone::Danger, e.clone(), None)),
            None => panel,
        }
    }

    fn share_invite(&mut self, ip: &str, port: Option<u16>, cx: &mut Context<Self>) -> Div {
        let mut d = div().flex().flex_col().gap(px(10.));
        if self.internet.token().is_none() {
            d = d.child(hint(tr(
                "Share this computer with a friend: they get it in their own Tailscale, and nothing else of yours. Do it in Tailscale's admin console (Machines, this computer, Share), or here with an access token you make there.",
            )));
            if !self.internet.asking_token {
                return d.child(
                    // The second button goes under the first when the card is narrow, rather
                    // than past its edge.
                    div()
                        .flex()
                        .flex_wrap()
                        .gap(px(6.))
                        .child(link(
                            "share-console",
                            "Open the admin console",
                            "https://login.tailscale.com/admin/machines",
                        ))
                        .child(self.share_button(
                            "share-token",
                            tr("Use an access token…"),
                            true,
                            cx,
                            |this, _| {
                                this.internet.asking_token = true;
                                this.internet.message = None;
                            },
                        )),
                );
            }
            // As tall as the token field they stand beside.
            let save = self
                .share_button("share-token-save", tr("Save"), true, cx, |this, _| {
                    this.save_token()
                })
                .h(px(tokens::FIELD));
            let keys = link("share-keys", "Make a token", tailscale::KEYS).h(px(tokens::FIELD));
            return d
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap(px(6.))
                        .child(self.share_field(Field::Token, "tskey-api-…", 280., cx))
                        .child(save)
                        .child(keys),
                )
                .child(hint(tr(
                    "Settings, Keys, Generate access token. It's kept in this computer's keychain and only sent to Tailscale.",
                )));
        }

        let idle = !self.internet.busy;
        match &self.internet.policy {
            Some(Ok((policy, _))) if tailscale::shares_entire_computer(policy) => {
                d = d.child(banner(Tone::Danger, tr(
                    "Friends you share this computer with can reach all of it, not only Minecraft.",
                ), None));
            }
            Some(Err(e)) => {
                d = d.child(hint(trf(
                    "Couldn't read your access controls: {error}",
                    &[("error", e)],
                )));
            }
            _ => {}
        }
        if port.is_none() {
            d = d.child(hint(tr(
                "The game port is unavailable, so Sunrise can't check access to that port.",
            )));
        }
        d = d
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(6.))
                    .child(self.share_field(Field::Email, tr("Their email (optional)"), 260., cx))
                    .child(self.share_button(
                        "share-invite",
                        tr("Make an invitation"),
                        idle,
                        cx,
                        |this, _| this.invite_friend(),
                    )
                    // As tall as the email field beside it.
                    .h(px(tokens::FIELD))),
            )
            .child(hint(tr(
                "One invitation per friend. With an email Tailscale sends it; without, send them the link. They accept it in their own Tailscale account.",
            )));
        if let Some(fresh) = self.internet.fresh.clone() {
            d = d.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .font_family(theme::MONO)
                            .text_size(px(12.))
                            .child(fresh.url.clone()),
                    )
                    .child(self.share_copy("share-fresh-copy", fresh.url, cx)),
            );
        }
        match self.internet.invites.clone() {
            Some(Ok(list)) => {
                for (n, inv) in list.into_iter().enumerate() {
                    let who = match (&inv.accepted_by, &inv.email) {
                        (Some(by), _) => trf("Shared with {who}", &[("who", by)]),
                        (None, Some(email)) => {
                            trf("Sent to {email}, not accepted yet", &[("email", email)])
                        }
                        (None, None) => tr("Link not used yet").to_string(),
                    };
                    let id = inv.id.clone();
                    let withdraw = self.share_button(
                        ElementId::named_usize("share-revoke", n),
                        tr(if inv.accepted {
                            "Stop sharing"
                        } else {
                            "Withdraw"
                        }),
                        idle,
                        cx,
                        move |this, _| {
                            let id = id.clone();
                            this.internet.fresh = None;
                            this.internet_action(move |api| {
                                api.revoke(&id)?;
                                Ok(tr(
                                    "Withdrawn: that friend can't reach this computer any more.",
                                )
                                .into())
                            });
                        },
                    );
                    let date = inv.created.get(..10).unwrap_or_default().to_string();
                    d = d.child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(12.))
                            .pt(px(8.))
                            .border_t_1()
                            .border_color(palette().rule())
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .gap(px(2.))
                                    .child(div().font_weight(FontWeight::SEMIBOLD).child(who))
                                    .child(hint(trf("Made {date}", &[("date", &date)]))),
                            )
                            .child(withdraw),
                    );
                }
            }
            Some(Err(e)) => d = d.child(banner(Tone::Danger, e, None)),
            None => {}
        }

        // What shared friends reach.
        let reach = match &self.internet.policy {
            Some(Ok((policy, etag))) => port.map(|port| {
                (
                    tailscale::reach(policy, ip, port),
                    policy.clone(),
                    etag.clone(),
                    port,
                )
            }),
            _ => None,
        };
        if let Some((reach, policy, etag, port)) = reach {
            let at = format!("{ip}:{port}");
            let safety = match reach {
                Reach::Everything if self.internet.confirm_limit => {
                    let new = tailscale::limit_shared(&policy, ip, port);
                    div()
                        .child(hint(trf(
                            "This changes your tailnet's access controls: the rule that lets everyone reach everything becomes one for your own devices and members, and friends you share with reach only {address}.",
                            &[("address", &at)],
                        )))
                        .child(
                            div()
                                .flex()
                                .gap(px(6.))
                                .child(self.share_button(
                                    "share-limit-no",
                                    tr("Leave them as they are"),
                                    true,
                                    cx,
                                    |this, _| this.internet.confirm_limit = false,
                                ))
                                .child(danger(self.share_button(
                                    "share-limit-go",
                                    tr("Change the access controls"),
                                    true,
                                    cx,
                                    move |this, _| {
                                        if let Some(new) = new.clone() {
                                            this.change_policy(
                                                new,
                                                etag.clone(),
                                                tr("Done: friends you share this computer with reach only the Minecraft server."),
                                            );
                                        }
                                    },
                                ))),
                        )
                }
                Reach::Everything => div().flex().child(self.share_button(
                    "share-limit",
                    tr("Limit them to Minecraft…"),
                    idle,
                    cx,
                    |this, _| this.internet.confirm_limit = true,
                )),
                Reach::ServerOnly => {
                    let undo = tailscale::unlimit_shared(&policy, ip, port);
                    div().child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(12.))
                            .child(
                                div()
                                    .flex_none()
                                    .size(px(8.))
                                    .rounded(crate::tokens::r(4.))
                                    .bg(rgb(palette().success)),
                            )
                            .child(div().flex_1().min_w_0().child(crate::selectable::selectable(trf(
                                "Friends you share with reach only the Minecraft server (port {port}).",
                                &[("port", &port.to_string())],
                            ))))
                            .child(self.share_button(
                                "share-unlimit",
                                tr("Remove the limit"),
                                idle,
                                cx,
                                move |this, _| {
                                    if let Some(undo) = undo.clone() {
                                        this.change_policy(
                                            undo,
                                            etag.clone(),
                                            tr("Tailscale's default access controls are back."),
                                        );
                                    }
                                },
                            )),
                    )
                }
                Reach::Custom => {
                    let snippet = tailscale::snippet(ip, port);
                    div()
                        .child(crate::selectable::selectable(tr(
                            "Your access controls are your own, so Sunrise leaves them alone. To keep shared friends to Minecraft, add this rule in the admin console:",
                        )))
                        .child(crate::code_box::code_box(
                            "share-acl-snippet",
                            vec![snippet.clone().into()],
                            None,
                        ))
                        .child(
                            div()
                                .flex()
                                .gap(px(6.))
                                .child(self.share_copy("share-snippet-copy", snippet, cx))
                                .child(link(
                                    "share-acls",
                                    "Open access controls",
                                    tailscale::ACCESS,
                                )),
                        )
                }
            };
            d = d.child(safety.flex().flex_col().gap(px(8.)).pt(px(4.)));
        }
        d.child(div().flex().child(self.share_button(
            "share-forget",
            tr("Forget the access token"),
            true,
            cx,
            |this, _| this.forget_token(),
        )))
    }
}
