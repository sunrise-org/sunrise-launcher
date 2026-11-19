//! The GUI bridge: preparation off the UI thread, one consent for the complete missing set.
use crate::frame::button;
use crate::*;
use std::{
    path::PathBuf,
    process::Command,
    sync::mpsc::{self, Sender},
};
use sunrise_launcher::{
    browser::sunrise,
    jars::{self, Permission},
    launch::{self, Launch},
};
use sunrise_launcher_smod::manifest::JarRef;

pub fn client(
    mut command: Command,
    tag: String,
    launch: Launch,
    run: Option<String>,
    account: Option<String>,
    session_file: Option<PathBuf>,
    tx: Sender<Msg>,
) {
    std::thread::spawn(move || {
        let mut registry_notices = Vec::new();
        let prepared = sunrise::before_play(&mut command, &tag, &launch).and_then(|notices| {
            registry_notices = notices;
            with_consent(&tx, |permission| {
                jars::prepare_launch(&mut command, &launch, permission)
            })
            .map_err(|error| error.to_string())?;
            launch::wrap(command, &launch)
        });
        if let Err(mpsc::SendError(Msg::JarClient { session_file, .. })) = tx.send(Msg::JarClient {
            run,
            account,
            session_file,
            registry_notices,
            prepared: Box::new(prepared),
        }) {
            sunrise_launcher::account::discard_launch_session(session_file);
        }
    });
}

/// Used only from background jobs: the UI owns the one complete-set prompt.
pub fn with_consent<T>(tx: &Sender<Msg>, prepare: impl FnOnce(Permission<'_>) -> T) -> T {
    let ask = |missing: &[&JarRef]| {
        let (reply, answer) = mpsc::channel();
        let jars = missing.iter().map(|jar| (*jar).clone()).collect();
        tx.send(Msg::JarConsent { jars, reply }).is_ok() && answer.recv().unwrap_or(false)
    };
    prepare(Permission::Ask(&ask))
}

pub(super) struct Consent {
    jars: Vec<JarRef>,
    reply: Sender<bool>,
}

impl Consent {
    pub(super) fn new(jars: Vec<JarRef>, reply: Sender<bool>) -> Self {
        Self { jars, reply }
    }

    pub(super) fn answer(self, allow: bool) {
        let _ = self.reply.send(allow);
    }

    /// Replace the OS prompt with one bounded list. All files covered by the answer are here.
    pub(super) fn sheet(&self, cx: &mut Context<Launcher>) -> Stateful<Div> {
        let count = self.jars.len();
        let heading = if count == 1 {
            tr("Download 1 original Java jar?").into()
        } else {
            trf(
                "Download {count} original Java jars?",
                &[("count", &count.to_string())],
            )
        };
        let body = div().flex().flex_col()
            .child(crate::sheets::title(heading).flex_none())
            .child(div().flex_none().text_size(px(tokens::LABEL)).child(tr(
                "Downloaded from Modrinth. Only assets and data are used; Java code is never run. Each file is checked against the manifest's SHA-512.",
            )))
            .child(div().id("jar-inventory").min_h_0().max_h(px(256.)).overflow_y_scroll()
                .track_scroll(&crate::frame::scrolls("jar-inventory"))
                .child(div().flex().flex_col().gap(px(8.)).children(self.jars.iter().enumerate().map(|(n, jar)| {
                    let identity = trf("{id} {version}", &[("id", jar.id()), ("version", jar.version())]);
                    let exact = jar.sha512().to_owned();
                    div().flex().flex_none().flex_col().gap(px(6.)).p(px(12.))
                        .border_1().border_color(gpui::rgb(crate::paint::palette().line))
                        .rounded(tokens::r(tokens::R_CARD))
                        .child(div().flex().items_center().gap(px(10.))
                            .child(div().id(("jar-identity", n)).flex_1().min_w_0().truncate().font_weight(FontWeight::SEMIBOLD)
                                .tooltip(crate::glass_ui::tip(identity.clone())).child(identity))
                            .child(button(("jar-copy", n), tr("Copy SHA-512")).flex_none().on_click(move |_, _, cx| {
                                crate::widgets::copy_to_clipboard(cx, gpui::ClipboardItem::new_string(exact.clone()));
                            })))
                        .child(div().min_w_0().text_size(px(tokens::SMALL)).font_family(crate::theme::MONO)
                            .child(crate::selectable::Selectable::styled(("jar-digest", n), gpui::StyledText::new(digest_lines(jar)))))
                }))))
            .child(div().flex().flex_none().flex_wrap().justify_end().gap(px(10.))
                .child(button("jar-cancel", tr("Cancel")).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.close_sheet();
                    cx.notify();
                })))
                .child(button("jar-allow", tr("Download jars")).on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.answer_jars(true);
                    cx.notify();
                }))));
        crate::sheets::bounded_frame("jar-consent", 580., body, cx)
    }
}

/// JarRef already guarantees 128 ASCII hex digits. Use two fixed-width lines; Copy gives the
/// original value with no visual linebreaks.
fn digest_lines(jar: &JarRef) -> String {
    let (left, right) = jar.sha512().split_at(64);
    format!("{left}\n{right}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_displayed_digest_has_every_digit_and_the_reply_is_boolean() {
        let digest = "0123456789abcdef".repeat(8);
        let jar = JarRef::new("example".into(), "1.0".into(), digest.clone()).unwrap();
        let displayed = digest_lines(&jar);
        assert_eq!(
            displayed.lines().map(str::len).collect::<Vec<_>>(),
            [64, 64]
        );
        assert_eq!(displayed.replace('\n', ""), digest);
        for allow in [false, true] {
            let (reply, answer) = mpsc::channel();
            Consent::new(vec![jar.clone()], reply).answer(allow);
            assert_eq!(answer.try_recv().unwrap(), allow);
        }
    }
}
