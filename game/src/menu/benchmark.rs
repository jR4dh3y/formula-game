//! Launches the same executable as an isolated benchmark, preserving the race.
use super::*;
use std::{
    fs::File,
    path::PathBuf,
    process::{Child, Command, Stdio},
};

#[derive(Default)]
pub(super) struct Benchmark {
    pub(super) child: Option<Child>,
    pub(super) output: Option<PathBuf>,
    pub(super) message: Option<String>,
}

impl Drop for Benchmark {
    fn drop(&mut self) {
        // Portable builds cannot remove their extracted EXE/assets until the
        // benchmark process has exited too. Let an in-flight run finish.
        if let Some(mut child) = self.child.take() {
            if let Err(error) = child.wait() {
                error!("Could not wait for benchmark shutdown: {error}");
            }
        }
    }
}

impl Benchmark {
    pub(super) fn poll(&mut self) {
        let Some(child) = &mut self.child else { return };
        let status = match child.try_wait() {
            Ok(None) => return,
            Ok(Some(status)) => status,
            Err(error) => {
                self.message = Some(format!("Could not read benchmark status: {error}"));
                return;
            }
        };
        self.child = None;
        self.message = Some(if status.success() {
            "Benchmark complete. Frame times and six screenshots are saved below.".into()
        } else {
            "Benchmark failed. See runtime.log in the output folder for details.".into()
        });
    }

    fn launch(
        &mut self,
        draft: &Options,
        graphics: crate::graphics::Quality,
        size: egui::Vec2,
    ) -> Result<(), String> {
        let cache = std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
            .ok_or("No cache directory available")?;
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos();
        let output = cache
            .join("f1-racecraft/benchmarks")
            .join(stamp.to_string());
        std::fs::create_dir_all(&output).map_err(|error| error.to_string())?;
        let log = File::create(output.join("runtime.log")).map_err(|error| error.to_string())?;
        #[cfg(not(test))]
        let executable = std::env::current_exe().map_err(|error| error.to_string())?;
        #[cfg(test)]
        let executable = std::env::var_os("F1_TEST_GAME_BINARY")
            .map(PathBuf::from)
            .ok_or("F1_TEST_GAME_BINARY is required for a renderer test launch")?;
        let mut command = Command::new(executable);
        command
            .args(arguments(draft, graphics, size, &output))
            .stdin(Stdio::null())
            .stdout(Stdio::from(
                log.try_clone().map_err(|error| error.to_string())?,
            ))
            .stderr(Stdio::from(log));
        self.child = Some(command.spawn().map_err(|error| error.to_string())?);
        self.output = Some(output);
        self.message = Some("Running in a separate window. Timing takes 20 seconds after loading and warm-up, followed by screenshot capture.".into());
        Ok(())
    }
}

fn arguments(
    draft: &Options,
    graphics: crate::graphics::Quality,
    size: egui::Vec2,
    output: &Path,
) -> Vec<std::ffi::OsString> {
    let camera = match draft.camera {
        Mode::Cockpit => "cockpit",
        Mode::Tv => "tv",
        Mode::Chase => "chase",
    };
    let mut args = vec![
        "--assets".into(),
        draft.assets.as_os_str().to_owned(),
        "--track".into(),
        draft.track.clone().into(),
        "--car".into(),
        draft.car.clone().into(),
        "--camera".into(),
        camera.into(),
        "--graphics".into(),
        graphics.label().to_ascii_lowercase().into(),
        "--size".into(),
        format!("{}x{}", size.x.round() as u32, size.y.round() as u32).into(),
        "--mute".into(),
        "--benchmark".into(),
        output.as_os_str().to_owned(),
        "--benchmark-seed".into(),
        draft.benchmark_seed.to_string().into(),
        "--benchmark-start".into(),
        draft.benchmark_start.to_string().into(),
    ];
    if !draft.team.is_empty() {
        args.extend(["--team".into(), draft.team.clone().into()]);
    }
    if !draft.driver.is_empty() {
        args.extend(["--driver".into(), draft.driver.clone().into()]);
    }
    args
}

pub(super) fn panel(ui: &mut egui::Ui, frontend: &mut Frontend, size: egui::Vec2) {
    section(ui, "Repeatable performance test");
    ui.label(
        "Two computer-controlled cars, already moving. A fixed 20-second drive with no contact.",
    );
    ui.add_space(12.);
    let available = frontend
        .catalog
        .circuits
        .iter()
        .any(|track| track.id == frontend.draft.track && track.unavailable.is_none());
    let run = ui.add_enabled(
        available && frontend.benchmark.child.is_none(),
        egui::Button::new(RichText::new("Run 20-second benchmark").strong())
            .fill(RED)
            .min_size(egui::vec2(ui.available_width().min(320.), 44.)),
    );
    #[cfg(test)]
    frontend.widgets.insert("benchmark-run", run.rect);
    if run.clicked() {
        if let Err(error) = frontend
            .benchmark
            .launch(&frontend.draft, frontend.graphics, size)
        {
            frontend.benchmark.message = Some(format!("Could not start benchmark: {error}"));
        }
    }
    ui.add_space(12.);
    egui::Grid::new("benchmark-summary")
        .spacing([28., 8.])
        .show(ui, |ui| {
            for (name, value) in [
                ("Circuit", frontend.draft.track.clone()),
                ("Car", frontend.draft.car.to_uppercase()),
                ("Camera", frontend.draft.camera.label().to_owned()),
                ("Graphics", frontend.graphics.label().to_owned()),
                (
                    "Resolution",
                    format!("{} × {}", size.x.round(), size.y.round()),
                ),
            ] {
                ui.label(RichText::new(name).color(DIM));
                ui.label(value);
                ui.end_row();
            }
        });
    ui.add_space(12.);
    ui.label("Choose the circuit and car in Race setup, the view in Cameras & controls, and the preset in Graphics. VSync is disabled for the test.");
    ui.label("Six seeded screenshot times cover the drive. Images are rendered after timing so capture does not distort the frame times.");
    ui.add_space(16.);
    if let Some(message) = &frontend.benchmark.message {
        ui.add_space(12.);
        ui.label(message);
    }
    if let Some(output) = &frontend.benchmark.output {
        ui.label(RichText::new("Results folder").strong());
        ui.add(
            egui::Label::new(output.display().to_string())
                .wrap()
                .selectable(true),
        );
        if frontend.benchmark.child.is_none() {
            if let Ok(bytes) = std::fs::read(output.join("benchmark.json")) {
                if let Ok(report) = serde_json::from_slice::<Report>(&bytes) {
                    ui.label(format!(
                        "{:.1} average FPS · {:.1} FPS 1% low · {:.2} ms p95",
                        report.metrics.average_fps,
                        report.metrics.one_percent_low_fps,
                        report.metrics.p95_ms
                    ));
                }
            }
        }
    }
}

#[derive(Deserialize)]
struct Report {
    metrics: Metrics,
}
#[derive(Deserialize)]
struct Metrics {
    average_fps: f64,
    one_percent_low_fps: f64,
    p95_ms: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn menu_launch_uses_selected_settings_and_the_normal_cli_contract() {
        let mut draft = Options::parse(std::iter::empty()).unwrap();
        draft.track = if cfg!(feature = "exp") { "cota" } else { "spa" }.into();
        draft.car = "gen22".into();
        draft.team = "ferrari".into();
        draft.driver = "leclerc".into();
        draft.camera = Mode::Cockpit;
        let args = arguments(
            &draft,
            crate::graphics::Quality::Ultra,
            egui::vec2(958., 1078.),
            Path::new("/tmp/benchmark output"),
        );
        let parsed =
            Options::parse(args.into_iter().map(|arg| arg.into_string().unwrap())).unwrap();
        assert_eq!(parsed.track, draft.track);
        assert_eq!(parsed.car, "gen22");
        assert_eq!(parsed.team, draft.team);
        assert_eq!(parsed.driver, draft.driver);
        assert_eq!(parsed.camera, Mode::Cockpit);
        assert_eq!((parsed.width, parsed.height), (958, 1078));
        assert_eq!(parsed.graphics, crate::graphics::Quality::Ultra);
        assert_eq!(parsed.opponents, 1);
        assert_eq!(
            parsed.benchmark.unwrap(),
            Path::new("/tmp/benchmark output")
        );
        assert!(!parsed.vsync);
    }

    #[test]
    fn shutdown_waits_for_the_benchmark_child() {
        use std::{sync::mpsc, time::Duration};
        let marker = std::env::temp_dir().join(format!(
            "racecraft-benchmark-shutdown-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "menu::benchmark::tests::shutdown_child_probe",
                "--ignored",
            ])
            .env("F1_BENCHMARK_SHUTDOWN_MARKER", &marker)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let release = child.stdin.take().unwrap();
        let benchmark = Benchmark {
            child: Some(child),
            output: None,
            message: None,
        };
        let (done, completed) = mpsc::channel();
        let waiter = std::thread::spawn(move || {
            drop(benchmark);
            done.send(()).unwrap();
        });
        // The child cannot finish until its input pipe closes. Drop must wait,
        // rather than just detach the child and return immediately.
        assert!(matches!(
            completed.recv_timeout(Duration::from_millis(100)),
            Err(mpsc::RecvTimeoutError::Timeout)
        ));
        drop(release);
        completed.recv_timeout(Duration::from_secs(30)).unwrap();
        waiter.join().unwrap();
        assert_eq!(std::fs::read(&marker).unwrap(), b"finished");
        std::fs::remove_file(marker).unwrap();
    }

    #[test]
    #[ignore = "subprocess helper for shutdown_waits_for_the_benchmark_child"]
    fn shutdown_child_probe() {
        use std::io::Read;
        let Some(marker) = std::env::var_os("F1_BENCHMARK_SHUTDOWN_MARKER") else {
            return;
        };
        std::io::stdin().read_to_end(&mut Vec::new()).unwrap();
        std::fs::write(marker, b"finished").unwrap();
    }
}
