//! The work under way as the strip draws it: one row for each job, the same everywhere. The
//! first job is always shown; the others fold behind "2 more" and open under it.
use super::jobs::{Job, JobAct};
use super::*;
use crate::widgets::button::secondary;

impl Launcher {
    pub(super) fn jobs_strip(&self, jobs: &[Job], cx: &mut Context<Self>) -> Div {
        let open = self.jobs.open && jobs.len() > 1;
        let toggle = (jobs.len() > 1).then(|| {
            let label = if open {
                tr("Show less").to_string()
            } else {
                trf("{count} more", &[("count", &(jobs.len() - 1).to_string())])
            };
            secondary("jobs-more", label).on_click(cx.listener(|this, _, _, cx| {
                this.jobs.open ^= true;
                cx.notify();
            }))
        });
        let shown = if open { jobs.len() } else { 1 };
        let mut rows = jobs.iter().enumerate().take(shown);
        let first = rows.next().map(|(n, job)| self.job_row(n, job, toggle, cx));
        plate()
            .flex()
            .flex_col()
            .gap(px(10.))
            .children(first)
            .children(rows.map(|(n, job)| self.job_row(n, job, None, cx)))
    }

    /// One job: what it does and its step, how far it is and the time left, over its bar; then
    /// Open and Stop when it has them.
    fn job_row(
        &self,
        n: usize,
        job: &Job,
        extra: Option<Stateful<Div>>,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let show = job.show.map(|act| {
            secondary(format!("job-open-{}", n), tr("Open")).on_click(cx.listener(
                move |this, _, _, cx| {
                    this.job_act(act);
                    cx.notify();
                },
            ))
        });
        let stop = job.stop.map(|act| {
            let label = match act {
                JobAct::CancelSwitch(_) => tr("Cancel"),
                _ => tr("Stop"),
            };
            let stop = secondary(format!("job-stop-{}", n), label);
            if job.stopping {
                crate::widgets::disabled(stop)
            } else {
                stop.on_click(cx.listener(move |this, _, _, cx| {
                    this.job_act(act);
                    cx.notify();
                }))
            }
        });
        div()
            .id(gpui::ElementId::named_usize("job", n))
            .flex()
            .items_center()
            .gap(px(12.))
            .child(
                div()
                    .flex_none()
                    .size(px(8.))
                    .rounded_full()
                    .bg(rgb(palette().accent)),
            )
            .child(self.job_text(job))
            .children(show)
            .children(stop)
            .children(extra)
    }

    /// A job's words over its bar: "Installing Nov 17 · Downloading", and "42% · ETA: 2 min" at
    /// the end of the line.
    fn job_text(&self, job: &Job) -> Div {
        let line = match &job.step {
            Some(step) => trf("{what} · {step}", &[("what", &job.what), ("step", step)]),
            None => job.what.clone(),
        };
        let percent = job
            .fraction
            .map(|f| trf("{value}%", &[("value", &format!("{:.0}", f * 100.))]));
        let eta = job.eta.map(crate::eta::eta_label);
        let meta = match (percent, eta) {
            (Some(percent), Some(eta)) => Some(trf(
                "{status} · {eta}",
                &[("status", &percent), ("eta", &eta)],
            )),
            (percent, eta) => percent.or(eta),
        };
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(div().flex_1().min_w_0().truncate().child(line))
                    .children(meta.map(|meta| {
                        div()
                            .flex_none()
                            .text_color(rgb(palette().muted))
                            .child(meta)
                    })),
            )
            .child(crate::widgets::feedback::progress_bar(job.fraction))
    }
}
