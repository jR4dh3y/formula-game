#![cfg_attr(windows, windows_subsystem = "windows")]

use flate2::read::ZlibDecoder;
use std::{
    env,
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process::{Command, ExitCode},
    time::{SystemTime, UNIX_EPOCH},
};

const FOOTER: &[u8; 16] = b"F1RACECRAFT-PKG1";
const HEADER: &[u8; 8] = b"F1ASSET1";
const MAX_EXPANDED_BYTES: u64 = 4 * 1024 * 1024 * 1024;

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

struct TemporaryDirectory(PathBuf);

impl TemporaryDirectory {
    fn new() -> io::Result<Self> {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_nanos();
        for attempt in 0..64 {
            let path = env::temp_dir().join(format!(
                "f1-racecraft-{}-{stamp}-{attempt}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self(path)),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::other("Could not create a temporary game folder"))
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        // The game has exited before this guard drops, releasing its EXE/assets.
        if let Err(error) = fs::remove_dir_all(&self.0) {
            eprintln!("Could not remove {}: {error}", self.0.display());
        }
    }
}

fn number<const N: usize>(input: &mut impl Read) -> io::Result<[u8; N]> {
    let mut bytes = [0; N];
    input.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn safe_relative_path(name: &str) -> io::Result<&Path> {
    if name.contains(['\\', ':', '\0'])
        || name.split('/').any(|part| {
            part.is_empty() || part == "." || part == ".." || part.ends_with(['.', ' '])
        })
    {
        return Err(invalid("Invalid path in the bundled game data"));
    }
    Ok(Path::new(name))
}

fn extract(mut input: impl Read, destination: &Path) -> io::Result<()> {
    if &number::<8>(&mut input)? != HEADER {
        return Err(invalid("Invalid game data header"));
    }
    let count = u32::from_le_bytes(number(&mut input)?);
    if count == 0 || count > 10_000 {
        return Err(invalid("Invalid bundled file count"));
    }
    let mut total = 0u64;
    for _ in 0..count {
        let name_size = u32::from_le_bytes(number(&mut input)?) as usize;
        let size = u64::from_le_bytes(number(&mut input)?);
        total = total
            .checked_add(size)
            .filter(|total| *total <= MAX_EXPANDED_BYTES)
            .ok_or_else(|| invalid("Bundled game data is too large"))?;
        if name_size == 0 || name_size > 4096 {
            return Err(invalid("Invalid bundled path length"));
        }
        let mut bytes = vec![0; name_size];
        input.read_exact(&mut bytes)?;
        let name = std::str::from_utf8(&bytes).map_err(|_| invalid("Invalid UTF-8 path"))?;
        let path = destination.join(safe_relative_path(name)?);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut output = OpenOptions::new().write(true).create_new(true).open(path)?;
        if io::copy(&mut input.by_ref().take(size), &mut output)? != size {
            return Err(invalid("Incomplete bundled game file"));
        }
    }
    if input.read(&mut [0])? != 0 {
        return Err(invalid("Unexpected trailing game data"));
    }
    Ok(())
}

fn unpack(executable: &Path, destination: &Path) -> io::Result<()> {
    let mut input = File::open(executable)?;
    let length = input.metadata()?.len();
    if length < 24 {
        return Err(invalid("The game package is incomplete"));
    }
    input.seek(SeekFrom::End(-24))?;
    if &number::<16>(&mut input)? != FOOTER {
        return Err(invalid("The game package footer is missing"));
    }
    let compressed_size = u64::from_le_bytes(number(&mut input)?);
    let offset = length
        .checked_sub(24)
        .and_then(|n| n.checked_sub(compressed_size))
        .filter(|n| *n > 0)
        .ok_or_else(|| invalid("Invalid game package size"))?;
    input.seek(SeekFrom::Start(offset))?;
    extract(ZlibDecoder::new(input.take(compressed_size)), destination)?;
    for file in [
        "f1-the-game.exe",
        "assets/tracks/spa/track.json",
        "assets/tracks/spa/collision.bin",
        "assets/tracks/spa/visual.glb",
        "assets/cars/rb6/car.json",
        "assets/cars/rb6/car.glb",
        "assets/cars/rb22/car.json",
        "assets/cars/rb22/car.glb",
    ] {
        if !destination.join(file).is_file() {
            return Err(invalid("A required game file is missing"));
        }
    }
    Ok(())
}

fn run(log: &mut File, data: &Path) -> io::Result<i32> {
    let directory = TemporaryDirectory::new()?;
    writeln!(log, "Extracting to {}", directory.0.display())?;
    unpack(&env::current_exe()?, &directory.0)?;
    if env::args_os()
        .nth(1)
        .is_some_and(|arg| arg == "--launcher-verify")
    {
        writeln!(log, "Package verified")?;
        return Ok(0);
    }
    let mut command = Command::new(directory.0.join("f1-the-game.exe"));
    command
        .current_dir(&directory.0)
        .arg("--assets")
        .arg(directory.0.join("assets"))
        .args(env::args_os().skip(1))
        .stdout(log.try_clone()?)
        .stderr(log.try_clone()?);
    if env::var_os("F1_SETTINGS").is_none() {
        command.env("F1_SETTINGS", data.join("preferences.json"));
    }
    if env::var_os("XDG_CACHE_HOME").is_none() {
        command.env("XDG_CACHE_HOME", data);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: no extra console.
    }
    let status = command.status()?;
    let code = status.code().unwrap_or(1);
    writeln!(log, "Game exited with code {code}")?;
    Ok(code)
}

fn show_error(message: &str) {
    #[cfg(windows)]
    {
        #[link(name = "user32")]
        extern "system" {
            fn MessageBoxW(
                window: *mut std::ffi::c_void,
                text: *const u16,
                title: *const u16,
                kind: u32,
            ) -> i32;
        }
        let message: Vec<_> = message.encode_utf16().chain(Some(0)).collect();
        let title: Vec<_> = "F1 Racecraft".encode_utf16().chain(Some(0)).collect();
        unsafe {
            MessageBoxW(std::ptr::null_mut(), message.as_ptr(), title.as_ptr(), 0x10);
        }
    }
    #[cfg(not(windows))]
    eprintln!("{message}");
}

fn main() -> ExitCode {
    let data = env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(env::temp_dir)
        .join("f1-racecraft");
    let log_path = data.join(format!("launcher-{}.log", std::process::id()));
    let result = fs::create_dir_all(&data)
        .and_then(|()| File::create(&log_path))
        .and_then(|mut log| {
            let result = run(&mut log, &data);
            if let Err(error) = &result {
                let _ = writeln!(log, "Launcher error: {error}");
            }
            result
        });
    match result {
        Ok(0) => ExitCode::SUCCESS,
        result => {
            let details = match result {
                Ok(code) => format!("The game exited with code {code}."),
                Err(error) => error.to_string(),
            };
            // Verification is noninteractive so automated checks cannot hang on a dialog.
            if !env::args_os()
                .nth(1)
                .is_some_and(|arg| arg == "--launcher-verify")
            {
                show_error(&format!("{details}\n\nDetails: {}", log_path.display()));
            }
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn archive(name: &str, data: &[u8], declared_size: u64) -> Vec<u8> {
        let mut bytes = HEADER.to_vec();
        bytes.extend(1u32.to_le_bytes());
        bytes.extend((name.len() as u32).to_le_bytes());
        bytes.extend(declared_size.to_le_bytes());
        bytes.extend(name.as_bytes());
        bytes.extend(data);
        bytes
    }

    #[test]
    fn extracts_nested_unicode_paths_and_cleans_up() {
        let directory = TemporaryDirectory::new().unwrap();
        let path = directory.0.clone();
        let bytes = archive("assets/café/data.bin", b"contents", 8);
        extract(bytes.as_slice(), &path).unwrap();
        assert_eq!(
            fs::read(path.join("assets/café/data.bin")).unwrap(),
            b"contents"
        );
        drop(directory);
        assert!(!path.exists());
    }

    #[test]
    fn rejects_escaping_and_windows_special_paths() {
        for path in [
            "../escape",
            "/absolute",
            "C:/absolute",
            "a\\b",
            "a/../b",
            "a:stream",
            "a/",
            "a.",
            "a ",
        ] {
            assert!(safe_relative_path(path).is_err(), "accepted {path}");
        }
    }

    #[test]
    fn rejects_truncated_files_and_trailing_data() {
        for bytes in [archive("file", b"short", 10), archive("file", b"extra", 4)] {
            let directory = TemporaryDirectory::new().unwrap();
            assert!(extract(bytes.as_slice(), &directory.0).is_err());
        }
    }

    #[test]
    fn rejects_corrupt_footer() {
        let directory = TemporaryDirectory::new().unwrap();
        let executable = directory.0.join("broken.exe");
        fs::write(&executable, [0; 32]).unwrap();
        assert!(unpack(&executable, &directory.0).is_err());
    }

    #[test]
    fn refuses_to_overwrite_an_extracted_file() {
        let directory = TemporaryDirectory::new().unwrap();
        fs::write(directory.0.join("file"), b"original").unwrap();
        let bytes = archive("file", b"replacement", 11);
        assert!(extract(bytes.as_slice(), &directory.0).is_err());
        assert_eq!(fs::read(directory.0.join("file")).unwrap(), b"original");
    }

    #[test]
    fn rejects_excessive_expanded_size() {
        let directory = TemporaryDirectory::new().unwrap();
        let bytes = archive("file", b"", MAX_EXPANDED_BYTES + 1);
        assert!(extract(bytes.as_slice(), &directory.0).is_err());
    }

    #[test]
    fn rejects_a_damaged_compression_checksum() {
        let bytes = archive("file", b"contents", 8);
        let mut encoder =
            flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&bytes).unwrap();
        let mut compressed = encoder.finish().unwrap();
        *compressed.last_mut().unwrap() ^= 1;
        let directory = TemporaryDirectory::new().unwrap();
        assert!(extract(ZlibDecoder::new(compressed.as_slice()), &directory.0).is_err());
    }
}
