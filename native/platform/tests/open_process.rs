//! Hermetic process smoke: starts only copies of this fixture, never user-associated apps.
#[cfg(windows)]
mod smoke {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
    use tiez_native_platform::open::{launch, Handler, Receipt, Route, Target};
    use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
    use windows::Win32::System::Threading::{
        GetCurrentProcess, GetProcessHandleCount, OpenProcess, WaitForSingleObject,
        PROCESS_SYNCHRONIZE,
    };
    const FIXTURE: &str = "TieZ-编辑器 %20 ' & fixture.exe";
    const OBSERVED: &str = "observed.bin";
    fn encoded<'a>(args: impl IntoIterator<Item = &'a OsStr>) -> Vec<u8> {
        let args: Vec<Vec<u16>> = args
            .into_iter()
            .map(|arg| arg.encode_wide().collect())
            .collect();
        let mut bytes = (args.len() as u32).to_le_bytes().to_vec();
        for arg in args {
            bytes.extend_from_slice(&(arg.len() as u32).to_le_bytes());
            for unit in arg {
                bytes.extend_from_slice(&unit.to_le_bytes());
            }
        }
        bytes
    }
    struct Scratch(PathBuf);
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn handles() -> u32 {
        let mut count = 0;
        unsafe {
            GetProcessHandleCount(GetCurrentProcess(), &mut count).unwrap();
        }
        count
    }
    struct OwnedProcess(HANDLE);
    impl Drop for OwnedProcess {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
    fn received(receipt: Receipt, output: &Path, expected: &[u8]) {
        // Fixture processes may already have exited. When still present, wait on our own handle.
        let process = receipt.process_id.and_then(|id| unsafe {
            OpenProcess(PROCESS_SYNCHRONIZE, false, id)
                .ok()
                .map(OwnedProcess)
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        while !output.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(output.exists(), "fixture did not receive a launch");
        assert!(
            std::fs::read(output).unwrap() == expected,
            "target/count changed in actual process argv"
        );
        if let Some(process) = process {
            assert_eq!(
                unsafe { WaitForSingleObject(process.0, 5000) },
                WAIT_OBJECT_0
            );
        }
        std::fs::remove_file(output).unwrap();
    }
    pub fn run() {
        let executable = std::env::current_exe().unwrap();
        if executable.file_name() == Some(OsStr::new(FIXTURE)) {
            let args: Vec<_> = std::env::args_os().skip(1).collect();
            let bytes = encoded(args.iter().map(|arg| arg.as_os_str()));
            let directory = executable.parent().unwrap();
            let pending = directory.join("observed.pending");
            std::fs::write(&pending, bytes).unwrap();
            std::fs::rename(pending, directory.join(OBSERVED)).unwrap();
            return;
        }
        let scratch = Scratch(std::env::temp_dir().join(format!("TieZ-open-process-{}-{}",
            std::process::id(), SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos())));
        std::fs::create_dir(&scratch.0).unwrap();
        let fixture = scratch.0.join(FIXTURE);
        std::fs::copy(&executable, &fixture).unwrap();
        let file = scratch.0.join("中文 %20 ' & one.txt");
        std::fs::write(&file, b"original untouched\r\n").unwrap();
        let directory = scratch.0.join("目录 %20 ' & trailing");
        std::fs::create_dir(&directory).unwrap();
        let trailing = PathBuf::from(format!("{}\\", directory.display()));
        let output = scratch.0.join(OBSERVED);
        // Independent handle snapshots found only one-time ETW/Key allocations on first
        // CreateProcessW, with no retained Process/Thread objects. Warm the complete launch
        // path, not just sleep, before measuring strict per-launch resource stability.
        let cold_handles = handles();
        let warm = launch(Handler::Desktop(&fixture), Target::Path(&file)).unwrap();
        received(warm, &output, &encoded([file.as_os_str()]));
        let before = handles();
        let mut direct_count = 0;
        for _ in 0..3 {
            for target in [
                Target::Path(&file),
                Target::Path(&trailing),
                Target::Url("https://example.test/?q=\"x\"&v=%20&u=中文"),
                Target::Url("https://example.test/end\\"),
            ] {
                let expected = match target {
                    Target::Path(path) => encoded([path.as_os_str()]),
                    Target::Url(url) => encoded([OsStr::new(url)]),
                };
                let receipt = launch(Handler::Desktop(&fixture), target).unwrap();
                assert_eq!(receipt.route, Route::Desktop);
                assert!(receipt.process_id.is_some());
                received(receipt, &output, &expected);
                direct_count += 1;
                assert_eq!(
                    handles(),
                    before,
                    "process handle count changed; ownership needs investigation"
                );
            }
        }
        assert_eq!(
            handles(),
            before,
            "process handle count changed; ownership needs investigation"
        );
        // CreateProcessW alone accepted .cmd here. The explicit PE gate must reject scripts,
        // even a script renamed as .exe, without starting an interpreter or changing handlers.
        for name in [
            "not-an-executable.cmd",
            "uppercase.CMD",
            "script.bat",
            "renamed-script.exe",
        ] {
            let script = scratch.0.join(name);
            std::fs::write(&script, b"@echo off\r\nexit /b 42\r\n").unwrap();
            assert_eq!(
                launch(Handler::Desktop(&script), Target::Path(&file)).unwrap_err(),
                "open_handler_executable"
            );
        }
        assert!(!output.exists());
        assert_eq!(handles(), before, "rejected process launch leaked handles");
        // ShellExecuteEx launches only our known fixture executable; no associations are changed.
        let receipt = launch(Handler::System, Target::Path(&fixture)).unwrap();
        assert_eq!(receipt.route, Route::System);
        received(receipt, &output, &encoded(std::iter::empty()));
        assert_eq!(std::fs::read(&file).unwrap(), b"original untouched\r\n");
        // Explicit cleanup must succeed once all child processes and files have been released.
        std::fs::remove_dir_all(&scratch.0).unwrap();
        println!("fixture first-launch warmup: {cold_handles} -> {before} handles; all 12 measured launch counts unchanged");
        println!("native open process smoke: {direct_count} desktop launches, exact single argv, stable warm handle counts, script rejection, one controlled system launch; no associated-document or packaged-app success tested");
    }
}
fn main() {
    #[cfg(windows)]
    smoke::run();
}
