use super::jobs::JobAct;
use crate::frame::Page;
use crate::mods::home_tests::in_scratch_launcher;

const TAG: &str = "nightly-2026-11-20-9425651";

#[test]
fn a_game_download_is_one_job_that_a_page_switch_keeps_and_stop_winds_down() {
    in_scratch_launcher(
        "status_strip::tests::a_game_download_is_one_job_that_a_page_switch_keeps_and_stop_winds_down",
        |launcher| {
            launcher.installing = Some((TAG.into(), 0.4));
            launcher.install_stop = Some(Default::default());
            for page in [Page::Home, Page::Discover, Page::Hosting, Page::Settings] {
                launcher.navigate(page);
                let jobs = launcher.jobs();
                assert_eq!(jobs.len(), 1, "on {page:?}");
                let job = &jobs[0];
                assert_eq!(job.what, "Installing Nightly · Nov 20");
                assert_eq!(job.step.as_deref(), Some("Downloading"));
                assert_eq!(job.fraction, Some(0.4));
                assert_eq!(job.stop, Some(JobAct::StopGameDownload));
                assert!(!job.stopping);
            }
            launcher.job_act(JobAct::StopGameDownload);
            assert!(launcher.install_stop.as_ref().unwrap().is_stopped());
            let jobs = launcher.jobs();
            assert!(jobs[0].stopping);
            assert_eq!(jobs[0].step.as_deref(), Some("Stopping…"));
        },
    );
}

#[test]
fn storage_work_shows_as_jobs_and_a_count_does_not() {
    in_scratch_launcher(
        "status_strip::tests::storage_work_shows_as_jobs_and_a_count_does_not",
        |launcher| {
            launcher.storage_pending = true;
            assert!(
                launcher.jobs().is_empty(),
                "counting storage is not work to watch"
            );
            launcher.storage_clearing = true;
            launcher.importing_build = true;
            launcher.removing = vec![TAG.into()];
            let jobs = launcher.jobs();
            let named: Vec<_> = jobs.iter().map(|job| job.what.as_str()).collect();
            assert_eq!(
                named,
                [
                    "Removing Nightly · Nov 20…",
                    "Importing a game build",
                    "Clearing downloads"
                ]
            );
            assert!(jobs
                .iter()
                .all(|job| job.show == Some(JobAct::Go(Page::Settings)) && job.stop.is_none()));
            launcher.job_act(JobAct::Go(Page::Settings));
            assert_eq!(launcher.ui.page, Page::Settings);
        },
    );
}

#[test]
fn the_list_folds_again_once_one_job_is_left() {
    in_scratch_launcher(
        "status_strip::tests::the_list_folds_again_once_one_job_is_left",
        |launcher| {
            launcher.importing_build = true;
            launcher.storage_pending = true;
            launcher.storage_clearing = true;
            launcher.jobs.open = true;
            assert_eq!(launcher.jobs().len(), 2);
            assert!(launcher.jobs.open);
            launcher.importing_build = false;
            assert_eq!(launcher.jobs().len(), 1);
            assert!(!launcher.jobs.open);
        },
    );
}

#[test]
fn play_says_how_far_its_download_is_and_what_its_preparation_does() {
    in_scratch_launcher(
        "status_strip::tests::play_says_how_far_its_download_is_and_what_its_preparation_does",
        |launcher| {
            use sunrise_launcher::backend::PlayStep;
            launcher.starting_client = Some(TAG.into());
            launcher.play_step = Some(PlayStep::Download);
            launcher.play_download = Some(0.4);
            let jobs = launcher.jobs();
            assert_eq!(jobs.len(), 1);
            assert_eq!(jobs[0].what, "Starting the game");
            assert_eq!(jobs[0].step.as_deref(), Some("Downloading the game…"));
            assert_eq!(jobs[0].fraction, Some(0.4));
            // Its preparation names the preparer's own step (sunrise-org/sunrise-launcher#417).
            launcher.play_step = Some(PlayStep::Prepare);
            launcher.play_download = None;
            let jobs = launcher.jobs();
            let step = jobs[0].step.as_deref().unwrap_or_default();
            assert_ne!(step, "Getting ready…");
            assert!(!step.is_empty());
        },
    );
}

#[test]
fn starting_the_game_and_preparing_it_can_be_stopped_from_every_page() {
    in_scratch_launcher(
        "status_strip::tests::starting_the_game_and_preparing_it_can_be_stopped_from_every_page",
        |launcher| {
            launcher.starting_client = Some(TAG.into());
            launcher.native_cancellation = Some(Default::default());
            for page in [Page::Home, Page::Discover, Page::Settings] {
                launcher.navigate(page);
                let jobs = launcher.jobs();
                assert_eq!(jobs.len(), 1, "on {page:?}");
                assert_eq!(jobs[0].stop, Some(JobAct::StopStart));
                assert!(!jobs[0].stopping);
            }
            launcher.job_act(JobAct::StopStart);
            assert!(launcher.native_cancellation.as_ref().unwrap().is_stopped());
            let jobs = launcher.jobs();
            assert!(jobs[0].stopping);
            assert_eq!(jobs[0].step.as_deref(), Some("Stopping…"));

            launcher.starting_client = None;
            launcher.native_cancellation = None;
            launcher.preparing = true;
            launcher.prepare_stop = Some(Default::default());
            let jobs = launcher.jobs();
            assert_eq!(jobs[0].stop, Some(JobAct::StopPrepare));
            launcher.job_act(JobAct::StopPrepare);
            assert!(launcher.prepare_stop.as_ref().unwrap().is_stopped());
            assert!(launcher.jobs()[0].stopping);
        },
    );
}

#[test]
fn a_start_without_a_stop_of_its_own_offers_none() {
    in_scratch_launcher(
        "status_strip::tests::a_start_without_a_stop_of_its_own_offers_none",
        |launcher| {
            launcher.starting_client = Some(TAG.into());
            let jobs = launcher.jobs();
            assert_eq!(jobs.len(), 1);
            assert_eq!(jobs[0].stop, None);
        },
    );
}
