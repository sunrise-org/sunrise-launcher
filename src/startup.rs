use super::*;

// Windows' process-entry stack is only 1 MiB. Keep the entire launcher, including CLI
// dispatch, on an explicitly sized stack instead of constructing startup frames there.
// GPUI initializes OLE, creates windows and pumps their messages on this same thread:
// no COM or GUI state is constructed on the entry thread that waits for it.
// 16 MiB is a generous reserve, sixteen times the 1 MiB default that the startup regression
// test constrains the entry thread to; Windows commits a thread stack only as it grows.
#[cfg(windows)]
pub(super) fn run() {
    let launcher = std::thread::Builder::new()
        .name("launcher".into())
        .stack_size(16 * 1024 * 1024)
        .spawn(run_launcher)
        .unwrap_or_else(|error| {
            eprintln!(
                "{}",
                trf(
                    "Could not start the launcher: {error}",
                    &[("error", &error.to_string())],
                )
            );
            std::process::exit(1);
        });
    if let Err(payload) = launcher.join() {
        std::panic::resume_unwind(payload);
    }
}

// In particular, AppKit must stay on macOS's original main thread.
#[cfg(not(windows))]
pub(super) fn run() {
    run_launcher();
}

fn run_launcher() {
    sunrise_launcher_smod::holder::retain_capability();
    if let Err(error) = console::attach_for_cli() {
        eprintln!(
            "{}",
            trf(
                "Could not restore the launcher's terminal streams: {error}",
                &[("error", &error.to_string())],
            )
        );
        std::process::exit(1);
    }
    if let Some(code) = sunrise_launcher::relocation::run_worker() {
        std::process::exit(code);
    }
    // A broken pointer may open Settings with Retry/Use the default folder; failing to
    // acquire the lease is different. Never run a writer while Move owns the data roots.
    if let Err(error) = sunrise_launcher::data_location::startup() {
        eprintln!("{error}");
        std::process::exit(1);
    }
    // Windowless Windows installer commands, including the new binary's notice refresh, never
    // reach the CLI parser (so neither the MCP tools nor the completions list them).
    #[cfg(windows)]
    if let Some(code) = sunrise_launcher::installer::run_quiet() {
        std::process::exit(code);
    }
    // Do not copy earlier launchers' data into fallback roots for a broken configured location.
    if sunrise_launcher::data_location::startup_error().is_none() {
        sunrise_launcher::carry_over::at_start();
    }
    perf::process_started();
    backend::exe_path();
    if let Some(code) = sunrise_launcher::update_handoff::run() {
        std::process::exit(code);
    }
    #[cfg(windows)]
    backend::remove_replaced_binaries();
    backend::cleanup_launcher_updates();
    // What the launcher downloads goes through the player's proxy, if they set one: the parts
    // that build their own agents (`sunrise-port-pack`) are told before anything starts.
    sunrise_launcher::proxy::announce(sunrise_launcher::backend::Settings::load().proxy);
    // The setup wizard and `sunrise-launcher.exe uninstall` (what Control Panel runs) are windows,
    // not CLI commands: asked of the CLI parser first, `uninstall` was refused as an unknown
    // subcommand, with the error going to the console that Control Panel's start doesn't have.
    #[cfg(windows)]
    let setup_window = sunrise_launcher::installer::window_mode();
    #[cfg(not(windows))]
    let setup_window: Option<sunrise_launcher::setup_args::Mode> = None;
    #[cfg(windows)]
    if !cfg!(debug_assertions) && setup_window.is_none() {
        let _ = sunrise_launcher::installer::ensure_notices();
    }
    if setup_window.is_none() {
        if let Some(code) = cli::run() {
            std::process::exit(code);
        }
    }
    // The GPU's texture limit, which caps the pictures made from disk (`picture_cache`).
    sunrise_launcher::pictures::probe_gpu_limit();
    gpui_platform::application()
        .with_assets(blocks::Assets)
        // `move`: the closure owns `setup_window`; `run` wants it to outlive this frame.
        .run(move |cx: &mut App| {
            cx.text_system()
                .add_fonts(vec![
                    Cow::Borrowed(include_bytes!("../assets/fonts/Figtree-Regular.ttf")),
                    Cow::Borrowed(include_bytes!("../assets/fonts/Figtree-Medium.ttf")),
                    Cow::Borrowed(include_bytes!("../assets/fonts/Figtree-SemiBold.ttf")),
                    Cow::Borrowed(include_bytes!("../assets/fonts/Figtree-Bold.ttf")),
                    Cow::Borrowed(sunrise_launcher_theme::fonts::GEIST_MONO),
                    Cow::Borrowed(sunrise_launcher_theme::fonts::BRICOLAGE_REGULAR),
                    Cow::Borrowed(sunrise_launcher_theme::fonts::BRICOLAGE_MEDIUM),
                    Cow::Borrowed(sunrise_launcher_theme::fonts::BRICOLAGE_SEMIBOLD),
                    Cow::Borrowed(sunrise_launcher_theme::fonts::BRICOLAGE_BOLD),
                    Cow::Borrowed(sunrise_launcher_theme::fonts::BRICOLAGE_DISPLAY_BOLD),
                    Cow::Borrowed(sunrise_launcher_theme::fonts::BRICOLAGE_DISPLAY_SEMIBOLD),
                    Cow::Borrowed(sunrise_launcher_theme::fonts::BRICOLAGE_HEADING_SEMIBOLD),
                    Cow::Borrowed(sunrise_launcher_theme::fonts::GEIST_REGULAR),
                    Cow::Borrowed(sunrise_launcher_theme::fonts::GEIST_MEDIUM),
                    Cow::Borrowed(sunrise_launcher_theme::fonts::GEIST_SEMIBOLD),
                ])
                .expect("bundled fonts");
            cx.bind_keys(key_bindings());
            cx.on_action(|_: &Quit, cx| shut_down(cx));
            cx.on_app_quit(|_| {
                sunrise_launcher::process::owned::stop_all();
                async {}
            })
            .detach();
            // The key press is being handled inside the window, so the window can only be
            // removed once that update has finished.
            cx.on_action(|_: &CloseWindow, cx| {
                cx.defer(|cx| {
                    for window in cx.windows() {
                        let _ = window.update(cx, |_, window, _| window.remove_window());
                    }
                })
            });
            cx.set_menus(menus());
            // The setup exe (and `sunrise-launcher.exe uninstall`) opens its own window instead.
            #[cfg(windows)]
            if let Some(mode) = setup_window {
                setup_page::open(mode, cx);
                return;
            }
            #[cfg(not(windows))]
            let _ = setup_window;
            let extent = size(px(1280.), px(800.));
            let bounds = Bounds::centered(None, extent, cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(800.), px(600.))),
                    titlebar: Some(titlebar::options()),
                    #[cfg(target_os = "linux")]
                    window_decorations: Some(gpui::WindowDecorations::Client),
                    app_id: Some(identity::APP_ID.into()),
                    // GPUI draws a window without focus at most every 33 ms to save energy. A
                    // benchmark's window often starts without it (KDE keeps focus from windows
                    // a background process opens), and it measures drawing, not that limit.
                    inactive_frame_interval: if perf::Bench::wanted() {
                        None
                    } else {
                        WindowOptions::default().inactive_frame_interval
                    },
                    ..Default::default()
                },
                |window, cx| cx.new(|cx| crate::window_root::Shell::new(window, cx)),
            )
            .expect("window");
            cx.on_window_closed(|cx, _| shut_down(cx)).detach();
            cx.activate(true);
        });
}
