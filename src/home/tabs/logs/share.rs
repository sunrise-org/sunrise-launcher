//! Share log: an upload to mclo.gs that happens only after the player confirms, without their
//! home folder or names (`game_log`), and the link it leaves, which can be copied, opened or
//! deleted again.
use super::tail::plain_error;
use super::*;
use crate::widgets::motion_bits::copy_button;

impl Launcher {
    /// Uploads this run's lines, or the profile's `latest.log` when this session has none.
    pub(super) fn share_log(&mut self, home: std::path::PathBuf, cx: &mut Context<Self>) {
        let source = match self.log_is_run(&home) {
            true => Ok(self
                .prepare_log
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join("\n")
                + "\n"),
            false => Err(self
                .ui
                .home
                .tabs
                .logs
                .target(&home)
                .unwrap_or_else(|| home.join("logs/latest.log"))),
        };
        let logs = &mut self.ui.home.tabs.logs;
        logs.confirm = false;
        logs.sharing = true;
        let task = cx.background_executor().spawn(async move {
            let text = match source {
                Ok(text) => text,
                Err(path) => match std::fs::read(&path) {
                    Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
                    Err(e) => {
                        return Err(trf(
                            "Could not read the log: {error}",
                            &[("error", &plain_error(&e))],
                        )
                        .to_string())
                    }
                },
            };
            let game = sunrise_launcher::backend::game_home();
            let homes = [home.as_path(), game.as_path()];
            let names = game_log::names(&homes);
            game_log::share(
                &game_log::api(),
                &game_log::private(&text, dirs::home_dir().as_deref(), &names),
            )
        });
        self.finish_share(task, cx);
    }

    /// Deletes the log shared from here.
    fn unshare_log(&mut self, cx: &mut Context<Self>) {
        let Some(Ok(shared)) = self.ui.home.tabs.logs.shared.clone() else {
            return;
        };
        self.ui.home.tabs.logs.sharing = true;
        let task = cx
            .background_executor()
            .spawn(async move { game_log::unshare(&game_log::api(), &shared) });
        cx.spawn(async move |this, cx| {
            let done = task.await;
            let _ = this.update(cx, |this, cx| {
                let logs = &mut this.ui.home.tabs.logs;
                logs.sharing = false;
                logs.shared = match done {
                    Ok(()) => None,
                    Err(e) => Some(Err(e)),
                };
                cx.notify();
            });
        })
        .detach();
    }

    fn finish_share(&mut self, task: gpui::Task<Result<Shared, String>>, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            let done = task.await;
            let _ = this.update(cx, |this, cx| {
                let logs = &mut this.ui.home.tabs.logs;
                logs.sharing = false;
                logs.shared = Some(done);
                cx.notify();
            });
        })
        .detach();
    }

    /// What stands between the toolbar and the log while sharing: the question before an upload
    /// (what is removed, where it goes), then the link it left or why it failed.
    pub(super) fn share_panel(
        &self,
        home: &std::path::Path,
        cx: &mut Context<Self>,
    ) -> Option<Div> {
        let logs = &self.ui.home.tabs.logs;
        let column = || div().flex_none().flex().flex_col().gap(px(8.));
        if logs.confirm {
            let home = home.to_path_buf();
            return Some(
                column()
                    .child(banner(
                        Tone::Info,
                        tr("Upload this log to mclo.gs?"),
                        Some(
                            tr("mclo.gs is a paste service for Minecraft logs. Anyone with the link can read the log until it is deleted or unread for 90 days. Your home folder, user name and account names are taken out first.")
                                .into(),
                        ),
                    ))
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .child(secondary("home-logs-share-no", tr("Cancel")).on_click(
                                cx.listener(|this, _, _, cx| {
                                    this.ui.home.tabs.logs.confirm = false;
                                    cx.notify();
                                }),
                            ))
                            .child(primary("home-logs-share-yes", tr("Upload")).on_click(
                                cx.listener(move |this, _, _, cx| {
                                    this.share_log(home.clone(), cx);
                                    cx.notify();
                                }),
                            )),
                    ),
            );
        }
        match &logs.shared {
            _ if logs.sharing => Some(column().child(muted(tr("Sending…")))),
            Some(Ok(shared)) => Some(self.shared_link(column(), &shared.url, cx)),
            Some(Err(e)) => Some(column().child(banner(Tone::Danger, e.clone(), None))),
            None => None,
        }
    }

    /// The link a shared log has: its address, with Copy link, Open and Delete it.
    fn shared_link(&self, column: Div, url: &str, cx: &mut Context<Self>) -> Div {
        let open = url.to_owned();
        column
            .child(banner(
                Tone::Success,
                trf("Shared: {url}", &[("url", url)]),
                None,
            ))
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(copy_button(
                        "home-logs-link-copy",
                        tr("Copy link"),
                        url.to_owned(),
                    ))
                    .child(
                        secondary("home-logs-link-open", tr("Open"))
                            .on_click(move |_, _, cx| cx.open_url(&open)),
                    )
                    .child(
                        secondary("home-logs-unshare", tr("Delete it")).on_click(cx.listener(
                            |this, _, _, cx| {
                                this.unshare_log(cx);
                                cx.notify();
                            },
                        )),
                    ),
            )
    }
}
