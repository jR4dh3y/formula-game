use std::{path::PathBuf, process::Command};

/// Exercises real process teardown while the renderer is still starting up.
#[test]
#[ignore = "requires a Vulkan display and F1_TEST_ASSETS"]
fn early_cli_exit_paths() {
    let assets = std::env::var_os("F1_TEST_ASSETS").expect("set F1_TEST_ASSETS");
    let directory = std::env::temp_dir().join(format!(
        "f1-bevy-exit-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&directory).unwrap();
    let missing = directory.join("missing/capture.png");
    let capture = directory.join("first-frame.png");
    let cases: [(u32, Option<PathBuf>, i32); 4] = [
        (1, None, 0),
        (12, None, 0),
        (1, Some(capture.clone()), 0),
        (12, Some(missing), 1),
    ];
    for (frames, path, expected) in cases {
        let mut command = Command::new(env!("CARGO_BIN_EXE_f1-the-game"));
        command
            .args(["--assets"])
            .arg(&assets)
            .args(["--opponents", "0", "--size", "640x360", "--frames"])
            .arg(frames.to_string());
        if let Some(path) = path {
            command.arg("--capture").arg(path);
        }
        let output = command.output().expect("launch game");
        assert_eq!(
            output.status.code(),
            Some(expected),
            "frames={frames}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let png = std::fs::read(capture).unwrap();
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
}
