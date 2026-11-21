//! Native NSH shell entry point for Omobench.

/// A stderr logger for the diagnostics NSH logs, including the `nsh::fps` performance lines
/// that the N2/N3/N7/N8 acceptance runs record when `NSH_FPS=1`.
#[cfg(all(feature = "nsh-ui", any(target_os = "linux", target_os = "macos")))]
fn install_logger() {
    struct StderrLogger;

    impl log::Log for StderrLogger {
        fn enabled(&self, _metadata: &log::Metadata<'_>) -> bool {
            true
        }

        fn log(&self, record: &log::Record<'_>) {
            use std::io::Write;
            let _ = writeln!(
                std::io::stderr().lock(),
                "{:>5} {} {}",
                record.level(),
                record.target(),
                record.args()
            );
        }

        fn flush(&self) {}
    }

    static LOGGER: StderrLogger = StderrLogger;
    if log::set_logger(&LOGGER).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
}

#[cfg(all(feature = "nsh-ui", target_os = "linux"))]
struct MainThreadProbeGuard {
    source: Option<nsh_platform_linux::gtk4::glib::SourceId>,
    probe: std::rc::Rc<std::cell::RefCell<omo_app::host::main_thread_probe::MainThreadProbe>>,
}

#[cfg(all(feature = "nsh-ui", target_os = "linux"))]
impl MainThreadProbeGuard {
    /// Complete and flush the capture after the native event loop returns.
    fn finish(mut self) {
        if let Some(source) = self.source.take() {
            source.remove();
        }
        self.probe.borrow_mut().finish();
    }
}

#[cfg(all(feature = "nsh-ui", target_os = "linux"))]
impl Drop for MainThreadProbeGuard {
    fn drop(&mut self) {
        if let Some(source) = self.source.take() {
            source.remove();
        }
        self.probe.borrow().stop_writer();
    }
}

#[cfg(all(feature = "nsh-ui", target_os = "linux"))]
fn install_linux_main_thread_probe() -> Option<MainThreadProbeGuard> {
    use std::time::{Instant, SystemTime};

    use omo_app::host::main_thread_probe::{LOOP_PERIOD, MainThreadProbe};

    let probe = std::rc::Rc::new(std::cell::RefCell::new(MainThreadProbe::from_env()?));

    #[cfg(feature = "main-thread-probe-test")]
    let mut test_stall_ms = std::env::var("OMOBENCH_MAIN_THREAD_TEST_STALL_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| (1..=5_000).contains(value));

    let callback_probe = std::rc::Rc::clone(&probe);
    let mut previous_callback = None;
    let mut previous_wall_callback = None;
    let wall_started = probe.borrow().wall_started();
    let source = nsh_platform_linux::gtk4::glib::timeout_add_local(LOOP_PERIOD, move || {
        let now = Instant::now();
        let wall_now = SystemTime::now();
        let gap = previous_callback.map(|previous| now.saturating_duration_since(previous));
        let wall_gap =
            previous_wall_callback.and_then(|previous| wall_now.duration_since(previous).ok());
        let wall_elapsed = wall_now.duration_since(wall_started).unwrap_or_default();
        callback_probe
            .borrow()
            .record_callback(now, wall_elapsed, gap, wall_gap);
        previous_callback = Some(now);
        previous_wall_callback = Some(wall_now);

        #[cfg(feature = "main-thread-probe-test")]
        if let Some(delay_ms) = test_stall_ms.take() {
            std::thread::sleep(std::time::Duration::from_millis(delay_ms));
        }

        nsh_platform_linux::gtk4::glib::ControlFlow::Continue
    });
    Some(MainThreadProbeGuard {
        source: Some(source),
        probe,
    })
}

#[cfg(all(feature = "nsh-ui", target_os = "linux"))]
fn main() {
    use nsh_platform_linux::{AppOptions, AppSource, run_app};

    install_logger();
    // NSH configures the process environment before starting GTK. Install the
    // optional probe only after the main context begins dispatching so its
    // writer thread cannot race that initialization.
    let main_thread_probe = std::rc::Rc::new(std::cell::RefCell::new(None));
    let probe_slot = std::rc::Rc::clone(&main_thread_probe);
    nsh_platform_linux::gtk4::glib::idle_add_local_once(move || {
        *probe_slot.borrow_mut() = install_linux_main_thread_probe();
    });
    let _ = run_app(
        AppSource::Rust(omo_app::ui::root()),
        AppOptions {
            app_id: "com.omobench.studio".into(),
            title: "Omobench Studio".into(),
            size: (1440, 960),
            ..AppOptions::default()
        },
    );
    if let Some(probe) = main_thread_probe.borrow_mut().take() {
        probe.finish();
    }
}

#[cfg(all(feature = "nsh-ui", target_os = "macos"))]
fn main() {
    use nsh_platform_macos::{AppOptions, AppSource, run_app};

    install_logger();
    let _ = run_app(
        AppSource::Rust(omo_app::ui::root()),
        AppOptions {
            app_id: "com.omobench.studio".into(),
            title: "Omobench Studio".into(),
            size: (1440, 960),
            ..AppOptions::default()
        },
    );
}

#[cfg(not(all(feature = "nsh-ui", any(target_os = "linux", target_os = "macos"))))]
fn main() {
    eprintln!("omo-app UI is available on Linux and macOS with the `nsh-ui` feature");
}
