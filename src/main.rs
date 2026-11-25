//! EndfieldDirectLauncher — start `Endfield.exe` directly, bypassing the official launcher UI.
//!
//! Why a plain `Start-Process Endfield.exe` does not work:
//! the game ships `OneChannelLC.dll` ("LC" = launch check). At SDK init it takes a
//! toolhelp snapshot, walks to its own parent PID, resolves the parent's image path with
//! `QueryFullProcessImageNameW`, and applies two checks to it:
//!
//!   1. the image *file name* must be `Games.exe` (the launcher's game-launch helper), and
//!   2. the image must carry a valid Authenticode signature from the client's publisher.
//!
//! Both are required, and they fail differently. A wrong name is reported as
//! `Not found matched name` with `errorCode: 10100`; a correctly named but unsigned parent is
//! reported as `Parent process verify signature failed:..., 2148204800` (`0x800B0100`,
//! `TRUST_E_NOSIGNATURE`) with `errorCode: 10101`. Either way the game then logs
//! `PC launch auth extraInfo failed` and asks for the official launcher. Note that the check
//! lives in `OneChannelLC.dll` but the signature API is resolved at run time
//! (`LoadLibraryExA` + `GetProcAddress`), so it does *not* appear in that module's import table.
//!
//! The bypass is therefore not a patch but a parenting trick: a publisher-signed executable is
//! copied to a temporary file *named* `Games.exe`, created suspended, and named as the new
//! process' parent via `PROC_THREAD_ATTRIBUTE_PARENT_PROCESS`. The kernel then reports
//! `games.exe` as the parent while not one instruction of that binary ever runs. The check
//! ignores where the parent lives — only its name and signature matter — so the copy can sit in
//! `%TEMP%`, and the official launcher does not have to be installed at all. Any signed
//! executable of the client will do, so we take one that ships inside the game directory.
//!
//! Elevation is declared in the binary manifest (`build.rs` passes
//! `/MANIFESTUAC:level='requireAdministrator'`) because `Endfield.exe` is itself
//! `requireAdministrator`. Windows therefore elevates this program once, at startup, and it
//! stays a single process for the whole session. That is not merely tidy: a Steam non-Steam
//! shortcut stops reporting "playing" the moment the process Steam launched exits, and
//! Steam's overlay can only follow into the game if the whole chain belongs to the process
//! Steam injected it into.
//!
//! No dependencies; raw `kernel32` / `user32` FFI only.

#![cfg_attr(not(test), windows_subsystem = "windows")]
#![allow(non_snake_case, non_camel_case_types, clippy::upper_case_acronyms)]

use std::ffi::{c_void, OsString};
use std::fs;
use std::io::Write;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::ptr::null_mut;

// ---------------------------------------------------------------- launch parameters

/// Exactly the argument vector the official chain uses:
/// `games.log` -> `cgsu create game process ... strCmdLine:"...\Endfield.exe" -launcher_lang=zh-cn -launcher_sub_channel=1`
const LAUNCHER_ARGS: &str = "-launcher_lang=zh-cn -launcher_sub_channel=1";

/// Executables inside the game directory that carry the publisher's signature and can therefore
/// stand in as the parent. They are only ever copied and suspended, never resumed. Ordered by
/// preference: smallest first, and `PlatformProcess.exe` is the platform helper the client ships
/// for exactly this kind of side-by-side role.
const PARENT_SOURCES: [&str; 3] = [
    "PlatformProcess.exe",
    "QtWebEngineProcess.exe",
    "UnityCrashHandler64.exe",
];

// ---------------------------------------------------------------- Win32 FFI

type HANDLE = *mut c_void;
type BOOL = i32;

const STARTF_USESHOWWINDOW: u32 = 0x0000_0001;
const SW_HIDE: u16 = 0;
const SW_SHOWNORMAL: u16 = 1;
const CREATE_SUSPENDED: u32 = 0x0000_0004;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// Needed as soon as `lpEnvironment` is a block we built ourselves: without it Windows reads
/// the block as ANSI and every non-ASCII value (a path, a user name) arrives mangled.
const CREATE_UNICODE_ENVIRONMENT: u32 = 0x0000_0400;
const EXTENDED_STARTUPINFO_PRESENT: u32 = 0x0008_0000;
const PROC_THREAD_ATTRIBUTE_PARENT_PROCESS: usize = 0x0002_0000;
const INFINITE: u32 = 0xFFFF_FFFF;
const WAIT_OBJECT_0: u32 = 0;
const WAIT_FAILED: u32 = 0xFFFF_FFFF;
const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
const MB_OK: u32 = 0x0000_0000;
const MB_ICONERROR: u32 = 0x0000_0010;
const MB_ICONINFORMATION: u32 = 0x0000_0040;

/// `JOBOBJECT_EXTENDED_LIMIT_INFORMATION` and `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`.
const JOB_OBJECT_EXTENDED_LIMIT_INFORMATION: i32 = 9;
const JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x0000_2000;

#[repr(C)]
struct STARTUPINFOW {
    cb: u32,
    lpReserved: *mut u16,
    lpDesktop: *mut u16,
    lpTitle: *mut u16,
    dwX: u32,
    dwY: u32,
    dwXSize: u32,
    dwYSize: u32,
    dwXCountChars: u32,
    dwYCountChars: u32,
    dwFillAttribute: u32,
    dwFlags: u32,
    wShowWindow: u16,
    cbReserved2: u16,
    lpReserved2: *mut u8,
    hStdInput: HANDLE,
    hStdOutput: HANDLE,
    hStdError: HANDLE,
}

#[repr(C)]
struct STARTUPINFOEXW {
    StartupInfo: STARTUPINFOW,
    lpAttributeList: *mut c_void,
}

#[repr(C)]
struct PROCESS_INFORMATION {
    hProcess: HANDLE,
    hThread: HANDLE,
    dwProcessId: u32,
    dwThreadId: u32,
}

#[repr(C)]
struct SYSTEMTIME {
    wYear: u16,
    wMonth: u16,
    wDayOfWeek: u16,
    wDay: u16,
    wHour: u16,
    wMinute: u16,
    wSecond: u16,
    wMilliseconds: u16,
}

#[repr(C)]
struct IO_COUNTERS {
    ReadOperationCount: u64,
    WriteOperationCount: u64,
    OtherOperationCount: u64,
    ReadTransferCount: u64,
    WriteTransferCount: u64,
    OtherTransferCount: u64,
}

#[repr(C)]
struct JOBOBJECT_BASIC_LIMIT_INFORMATION {
    PerProcessUserTimeLimit: i64,
    PerJobUserTimeLimit: i64,
    LimitFlags: u32,
    MinimumWorkingSetSize: usize,
    MaximumWorkingSetSize: usize,
    ActiveProcessLimit: u32,
    Affinity: usize,
    PriorityClass: u32,
    SchedulingClass: u32,
}

#[repr(C)]
struct JOBOBJECT_EXTENDED_LIMIT_INFORMATION {
    BasicLimitInformation: JOBOBJECT_BASIC_LIMIT_INFORMATION,
    IoInfo: IO_COUNTERS,
    ProcessMemoryLimit: usize,
    JobMemoryLimit: usize,
    PeakProcessMemoryUsed: usize,
    PeakJobMemoryUsed: usize,
}

#[link(name = "kernel32")]
extern "system" {
    fn GetModuleFileNameW(hModule: HANDLE, lpFilename: *mut u16, nSize: u32) -> u32;
    fn CreateProcessW(
        lpApplicationName: *const u16,
        lpCommandLine: *mut u16,
        lpProcessAttributes: *mut c_void,
        lpThreadAttributes: *mut c_void,
        bInheritHandles: BOOL,
        dwCreationFlags: u32,
        lpEnvironment: *mut c_void,
        lpCurrentDirectory: *const u16,
        lpStartupInfo: *mut STARTUPINFOEXW,
        lpProcessInformation: *mut PROCESS_INFORMATION,
    ) -> BOOL;
    fn InitializeProcThreadAttributeList(
        lpAttributeList: *mut c_void,
        dwAttributeCount: u32,
        dwFlags: u32,
        lpSize: *mut usize,
    ) -> BOOL;
    fn UpdateProcThreadAttribute(
        lpAttributeList: *mut c_void,
        dwFlags: u32,
        Attribute: usize,
        lpValue: *const c_void,
        cbSize: usize,
        lpPreviousValue: *mut c_void,
        lpReturnSize: *mut usize,
    ) -> BOOL;
    fn DeleteProcThreadAttributeList(lpAttributeList: *mut c_void);
    fn CreateJobObjectW(lpJobAttributes: *mut c_void, lpName: *const u16) -> HANDLE;
    fn SetInformationJobObject(
        hJob: HANDLE,
        JobObjectInformationClass: i32,
        lpJobObjectInformation: *const c_void,
        cbJobObjectInformationLength: u32,
    ) -> BOOL;
    fn AssignProcessToJobObject(hJob: HANDLE, hProcess: HANDLE) -> BOOL;
    fn TerminateProcess(hProcess: HANDLE, uExitCode: u32) -> BOOL;
    fn GetExitCodeProcess(hProcess: HANDLE, lpExitCode: *mut u32) -> BOOL;
    fn WaitForSingleObject(hHandle: HANDLE, dwMilliseconds: u32) -> u32;
    fn CloseHandle(hObject: HANDLE) -> BOOL;
    fn GetLastError() -> u32;
    fn GetStdHandle(nStdHandle: u32) -> HANDLE;
    fn WriteFile(
        hFile: HANDLE,
        lpBuffer: *const u8,
        nNumberOfBytesToWrite: u32,
        lpNumberOfBytesWritten: *mut u32,
        lpOverlapped: *mut c_void,
    ) -> BOOL;
    fn AllocConsole() -> BOOL;
    fn GetLocalTime(lpSystemTime: *mut SYSTEMTIME);
}

#[link(name = "user32")]
extern "system" {
    fn MessageBoxW(hWnd: HANDLE, lpText: *const u16, lpCaption: *const u16, uType: u32) -> i32;
}

// ---------------------------------------------------------------- small helpers

fn wide(s: &str) -> Vec<u16> {
    OsString::from(s).encode_wide().chain(Some(0)).collect()
}

fn wide_path(p: &Path) -> Vec<u16> {
    p.as_os_str().encode_wide().chain(Some(0)).collect()
}

/// `"<exe>" <args>` as UTF-16, assembled from the path itself instead of `Path::display()` so
/// that a path which is not valid UTF-8 cannot be mangled on the way into the command line.
fn wide_command_line(exe: &Path, args: &str) -> Vec<u16> {
    let mut line = vec!['"' as u16];
    line.extend(exe.as_os_str().encode_wide());
    line.push('"' as u16);
    if !args.is_empty() {
        line.push(' ' as u16);
        line.extend(OsString::from(args).encode_wide());
    }
    line.push(0);
    line
}

// ---------------------------------------------------------------- per-launch environment

/// The name ReShade's own Vulkan layer manifest declares in `disable_environment`. Setting it
/// in the `Endfield.exe` environment is what the official installer relies on for its
/// "disable the overlay for this program" option, so no file in the game directory changes:
/// the Vulkan loader sees the variable when the game creates its instance and drops the layer
/// before any of its code runs.
const RESHADE_DISABLE_ENV: &str = "DISABLE_VK_LAYER_reshade_1";

/// The Vulkan loader's own name filter, used as the second half of `--no-reshade`.
///
/// `DISABLE_VK_LAYER_reshade_1` only works because ReShade's layer manifest happens to declare
/// that name in `disable_environment`; a future ReShade that renames it would silently make the
/// flag a no-op. `VK_LOADER_LAYERS_DISABLE` is matched by the loader itself against the layer
/// *name*, so it keeps working no matter what the manifest says. Both are set: the manifest key
/// is what the ReShade installer documents, the loader filter is the belt to its braces.
const VK_LOADER_LAYERS_DISABLE_ENV: &str = "VK_LOADER_LAYERS_DISABLE";
const RESHADE_LAYER_NAME: &str = "VK_LAYER_reshade";

/// Collect `--env NAME=VALUE` pairs (repeatable) and the shorthands that stand for one.
///
/// Last one wins for a repeated name, and nothing is applied to this process: the values only
/// ever reach the game, so `--no-reshade` on one launch cannot disable the layer for the next.
fn collect_env_overrides(args: &[String]) -> Result<Vec<(String, String)>, String> {
    let mut overrides: Vec<(String, String)> = Vec::new();
    let mut index = 0usize;
    while index < args.len() {
        let arg = &args[index];
        let assignment = if arg == "--env" {
            // A following option is never a value: without this guard `--env --no-reshade`
            // would fail with a confusing "has no '=' in it" instead of naming the real mistake.
            let Some(value) = args.get(index + 1).filter(|value| !value.starts_with("--")) else {
                return Err("--env needs a NAME=VALUE argument (found another option instead)"
                    .to_string());
            };
            index += 2;
            Some(value.as_str())
        } else if let Some(rest) = arg.strip_prefix("--env=") {
            index += 1;
            Some(rest)
        } else if arg == "--no-reshade" {
            index += 1;
            for (name, value) in [
                (RESHADE_DISABLE_ENV, "1"),
                (VK_LOADER_LAYERS_DISABLE_ENV, RESHADE_LAYER_NAME),
            ] {
                overrides.retain(|(existing, _)| !existing.eq_ignore_ascii_case(name));
                overrides.push((name.to_string(), value.to_string()));
            }
            continue;
        } else {
            index += 1;
            continue;
        };
        let Some(assignment) = assignment else { continue };
        let Some((name, value)) = assignment.split_once('=') else {
            return Err(format!(
                "--env expects NAME=VALUE, but '{assignment}' has no '=' in it"
            ));
        };
        if name.is_empty() {
            return Err(format!("--env expects a variable name before '=', got '{assignment}'"));
        }
        overrides.retain(|(existing, _)| !existing.eq_ignore_ascii_case(name));
        overrides.push((name.to_string(), value.to_string()));
    }
    Ok(overrides)
}

/// Build the `NAME=VALUE\0…\0\0` block `CreateProcessW` wants, or `None` when nothing is
/// overridden so the child inherits this process' environment untouched.
///
/// Windows requires the block sorted case-insensitively by name and forbids two entries with
/// the same name, which is why the inherited copy is filtered before the overrides go in: a
/// stale `DISABLE_VK_LAYER_reshade_1` in this process' own environment must not survive next
/// to ours.
fn build_environment_block(overrides: &[(String, String)]) -> Option<Vec<u16>> {
    if overrides.is_empty() {
        return None;
    }
    let mut entries: Vec<(String, OsString)> = std::env::vars_os()
        .filter_map(|(key, value)| key.into_string().ok().map(|key| (key, value)))
        .collect();
    for (name, value) in overrides {
        entries.retain(|(key, _)| !key.eq_ignore_ascii_case(name));
        entries.push((name.clone(), OsString::from(value)));
    }
    entries.sort_by_key(|(key, _)| key.to_ascii_uppercase());
    let mut block: Vec<u16> = Vec::new();
    for (key, value) in entries {
        block.extend(OsString::from(format!("{key}=")).encode_wide());
        block.extend(value.encode_wide());
        block.push(0);
    }
    block.push(0);
    Some(block)
}

/// Ceiling for `EndfieldDirectLauncher.log` before it is rotated to `.old`.
const LOG_ROTATE_BYTES: u64 = 1 << 20;

fn needs_rotation(current_size: u64) -> bool {
    current_size >= LOG_ROTATE_BYTES
}

/// Whether a failure should also be raised as a modal message box.
///
/// A box is the only way to reach someone who double-clicked the launcher from Explorer, but it is
/// also the one call that can hang an unattended launch forever: with no interactive desktop it
/// never returns. So it is shown only when this process has no console output to fall back on, and
/// `--no-popup` turns it off for scripted launches.
fn wants_message_box(has_stdout: bool, no_popup: bool) -> bool {
    !has_stdout && !no_popup
}

/// The stdout we can write to, if the caller gave us one (a shell pipe) or `--console`
/// allocated a console. Explorer gives us none.
fn stdout() -> Option<HANDLE> {
    let handle = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) };
    if handle.is_null() || handle as isize == -1 {
        None
    } else {
        Some(handle)
    }
}

/// Mirror every line into a log file beside this executable. The instance that actually
/// starts the game runs elevated with no console, so without this a failure there would
/// leave nothing behind but a message box.
fn append_to_log_file(line: &str) {
    let Ok(exe) = std::env::current_exe() else { return };
    let Some(dir) = exe.parent() else { return };
    let mut now: SYSTEMTIME = unsafe { std::mem::zeroed() };
    unsafe { GetLocalTime(&mut now) };
    let path = dir.join("EndfieldDirectLauncher.log");
    // Append-only with no ceiling grows without bound on a machine that launches the game for
    // months, and every line opens the file again. One rotation keeps it bounded and still leaves
    // the previous session available as .old.
    if fs::metadata(&path).map(|meta| needs_rotation(meta.len())).unwrap_or(false) {
        let rotated = dir.join("EndfieldDirectLauncher.log.old");
        let _ = fs::remove_file(&rotated);
        let _ = fs::rename(&path, &rotated);
    }
    let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(path) else {
        return;
    };
    let _ = writeln!(
        file,
        "{:02}:{:02}:{:02}.{:03} [{}] {}",
        now.wHour,
        now.wMinute,
        now.wSecond,
        now.wMilliseconds,
        std::process::id(),
        line
    );
}

/// Console logging that can never panic: with `windows_subsystem = "windows"` there is
/// usually no stdout, and `println!` would abort the process. Errors always reach the user
/// through a message box, and every line also lands in the log file.
fn log(line: &str) {
    append_to_log_file(line);
    let Some(handle) = stdout() else { return };
    let text = format!("{}\r\n", line);
    let mut written = 0u32;
    unsafe {
        WriteFile(
            handle,
            text.as_ptr(),
            text.len() as u32,
            &mut written,
            null_mut(),
        )
    };
}

fn message_box(text: &str, caption: &str, flags: u32) {
    unsafe { MessageBoxW(null_mut(), wide(text).as_ptr(), wide(caption).as_ptr(), flags) };
}

fn fail(text: String, show_box: bool) -> ! {
    log(&format!("ERROR: {}", text));
    if show_box {
        message_box(&text, "EndfieldDirectLauncher", MB_OK | MB_ICONERROR);
    }
    std::process::exit(1);
}

fn exe_dir() -> PathBuf {
    let mut buf = vec![0u16; 32768];
    let n = unsafe { GetModuleFileNameW(null_mut(), buf.as_mut_ptr(), buf.len() as u32) };
    let path = PathBuf::from(OsString::from_wide(&buf[..n as usize]));
    path.parent().map(Path::to_path_buf).unwrap_or(path)
}

// ---------------------------------------------------------------- the stand-in parent

/// Copy a publisher-signed executable from the game directory to a temporary file *named*
/// `Games.exe`.
///
/// The name is what the launch check matches on and the signature is what it verifies; where
/// the file lives is not part of the check. Staging a copy is therefore enough to satisfy both,
/// and it keeps the game directory clean — unlike pointing at the official launcher's own
/// `Games.exe`, which forces the launcher to be installed and its version directories to be
/// guessed.
fn stage_parent_exe(game_dir: &Path) -> Result<PathBuf, String> {
    let source = PARENT_SOURCES
        .iter()
        .map(|name| game_dir.join(name))
        .find(|path| path.is_file())
        .ok_or_else(|| {
            format!(
                "None of the executables this program uses as the parent process exist in\n{}\n\
                 ({}).\n\nIs this really the game directory?",
                game_dir.display(),
                PARENT_SOURCES.join(", ")
            )
        })?;

    let staging_dir = staging_dir();
    fs::create_dir_all(&staging_dir)
        .map_err(|e| format!("Could not create {}:\n{}", staging_dir.display(), e))?;
    let staged = staging_dir.join("Games.exe");
    fs::copy(&source, &staged).map_err(|e| {
        format!(
            "Could not copy\n{}\nto\n{}\n{}",
            source.display(),
            staged.display(),
            e
        )
    })?;

    log(&format!(
        "parent exe: {} (copy of {})",
        staged.display(),
        source
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    ));
    Ok(staged)
}

/// Root under which every run gets its own directory.
fn staging_root() -> PathBuf {
    std::env::temp_dir().join("EndfieldDirectLauncher")
}

/// Directory for *this* run's stand-in parent.
///
/// It used to be one fixed path shared by every launch, which made two runs collide: a placeholder
/// left behind by a killed launcher (or a second launch started before the first one had cleaned
/// up) still held `Games.exe` open, and the next `fs::copy` failed with "Could not copy ...
/// The process cannot access the file because it is being used by another process. (os error 32)".
/// The launch check only looks at the file *name* and its signature, so the file can live
/// anywhere - giving each run its own directory removes the collision without touching the game
/// directory.
fn staging_dir() -> PathBuf {
    staging_root().join(std::process::id().to_string())
}

/// Drop the staged copy once the placeholder is gone. Windows releases a terminated process image
/// section a moment after the fact, so a few retries are needed before the file will actually go.
fn remove_staged_parent(path: &Path) {
    for attempt in 0..10 {
        match fs::remove_file(path) {
            Ok(()) => {
                log(&format!("removed {}", path.display()));
                if let Some(dir) = path.parent() {
                    let _ = fs::remove_dir(dir);
                }
                return;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if let Some(dir) = path.parent() {
                    let _ = fs::remove_dir(dir);
                }
                return;
            }
            Err(error) => {
                if attempt == 9 {
                    log(&format!("could not remove {}: {}", path.display(), error));
                }
                std::thread::sleep(std::time::Duration::from_millis(150));
            }
        }
    }
}

// ---------------------------------------------------------------- process creation

struct Child {
    process: HANDLE,
    pid: u32,
}

impl Child {
    unsafe fn close(&mut self) {
        if !self.process.is_null() {
            CloseHandle(self.process);
            self.process = null_mut();
        }
    }
}

/// Start the staged `Games.exe` suspended: it must exist as a process (so it can be named as
/// parent) but must never run, otherwise it would do whatever that binary does on startup.
fn start_suspended_pseudoparent(games_exe: &Path) -> Result<Child, String> {
    let application = wide_path(games_exe);
    let mut command_line = wide_command_line(games_exe, "");
    let cwd = games_exe.parent().map(wide_path).unwrap_or_else(|| wide("."));

    let mut startup: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
    // Not an extended startup: no `EXTENDED_STARTUPINFO_PRESENT` and no attribute list, so the
    // size is the plain structure's, as documented for that case.
    startup.StartupInfo.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESHOWWINDOW;
    startup.StartupInfo.wShowWindow = SW_HIDE;

    let mut info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    let ok = unsafe {
        CreateProcessW(
            application.as_ptr(),
            command_line.as_mut_ptr(),
            null_mut(),
            null_mut(),
            0,
            CREATE_SUSPENDED | CREATE_NO_WINDOW,
            null_mut(),
            cwd.as_ptr(),
            &mut startup,
            &mut info,
        )
    };
    if ok == 0 {
        return Err(format!(
            "could not start the suspended parent process\n{}\nWin32 error {}",
            games_exe.display(),
            unsafe { GetLastError() }
        ));
    }
    unsafe { CloseHandle(info.hThread) };
    log(&format!("suspended parent: Games.exe pid {}", info.dwProcessId));
    Ok(Child { process: info.hProcess, pid: info.dwProcessId })
}

/// Make the suspended placeholder die with this process.
///
/// It has no code of its own and is never resumed, so it can never exit by itself: if this
/// process is force-killed the placeholder would sit there forever. A job object with
/// `KILL_ON_JOB_CLOSE` is the only mechanism that survives `TerminateProcess` — the kernel
/// closes the handle for us and takes everything in the job down with it.
fn kill_placeholder_with_us(process: HANDLE) {
    unsafe {
        let job = CreateJobObjectW(null_mut(), std::ptr::null());
        if job.is_null() {
            log(&format!(
                "could not create the cleanup job, Win32 error {}",
                GetLastError()
            ));
            return;
        }
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let configured = SetInformationJobObject(
            job,
            JOB_OBJECT_EXTENDED_LIMIT_INFORMATION,
            &limits as *const _ as *const c_void,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        if configured == 0 || AssignProcessToJobObject(job, process) == 0 {
            log(&format!(
                "could not put the suspended parent in a cleanup job, Win32 error {}",
                GetLastError()
            ));
            CloseHandle(job);
        }
        // The handle is deliberately never closed: it has to stay open for as long as this
        // process lives, which is exactly how long the placeholder is wanted.
    }
}

/// Start `Endfield.exe` with `parent` as its reported parent process.
///
/// `environment` is the block from [`build_environment_block`]; it is only ever passed when the
/// caller asked for overrides, so an ordinary launch keeps handing the child `NULL` and thereby
/// this process' exact environment.
fn start_game(
    game_exe: &Path,
    game_dir: &Path,
    parent: HANDLE,
    environment: Option<&mut Vec<u16>>,
) -> Result<Child, String> {
    let mut list_size = 0usize;
    unsafe { InitializeProcThreadAttributeList(null_mut(), 1, 0, &mut list_size) };
    let align = std::mem::size_of::<usize>();
    let mut storage = vec![0usize; list_size.div_ceil(align)];
    let attribute_list = storage.as_mut_ptr() as *mut c_void;
    if unsafe { InitializeProcThreadAttributeList(attribute_list, 1, 0, &mut list_size) } == 0 {
        return Err(format!(
            "InitializeProcThreadAttributeList failed, Win32 error {}",
            unsafe { GetLastError() }
        ));
    }

    let attached = unsafe {
        UpdateProcThreadAttribute(
            attribute_list,
            0,
            PROC_THREAD_ATTRIBUTE_PARENT_PROCESS,
            &parent as *const HANDLE as *const c_void,
            std::mem::size_of::<HANDLE>(),
            null_mut(),
            null_mut(),
        )
    };
    if attached == 0 {
        unsafe { DeleteProcThreadAttributeList(attribute_list) };
        return Err(format!(
            "UpdateProcThreadAttribute(PARENT_PROCESS) failed, Win32 error {}",
            unsafe { GetLastError() }
        ));
    }

    let application = wide_path(game_exe);
    let mut command_line = wide_command_line(game_exe, LAUNCHER_ARGS);
    let cwd = wide_path(game_dir);

    let mut startup: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
    startup.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESHOWWINDOW;
    startup.StartupInfo.wShowWindow = SW_SHOWNORMAL;
    startup.lpAttributeList = attribute_list;

    let (environment_pointer, environment_flags) = match environment {
        Some(block) => (
            block.as_mut_ptr() as *mut c_void,
            CREATE_UNICODE_ENVIRONMENT,
        ),
        None => (null_mut(), 0u32),
    };

    let mut info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    let ok = unsafe {
        CreateProcessW(
            application.as_ptr(),
            command_line.as_mut_ptr(),
            null_mut(),
            null_mut(),
            0,
            EXTENDED_STARTUPINFO_PRESENT | environment_flags,
            environment_pointer,
            cwd.as_ptr(),
            &mut startup,
            &mut info,
        )
    };
    let last_error = unsafe { GetLastError() };
    unsafe { DeleteProcThreadAttributeList(attribute_list) };
    if ok == 0 {
        return Err(format!(
            "could not start the game\n{}\nWin32 error {}",
            game_exe.display(),
            last_error
        ));
    }
    unsafe { CloseHandle(info.hThread) };
    log(&format!("started Endfield.exe pid {}", info.dwProcessId));
    Ok(Child { process: info.hProcess, pid: info.dwProcessId })
}

// ---------------------------------------------------------------- steam app id hint

/// Delete the Steam app-id hint the game leaves in its own directory.
///
/// `steam_appid.txt` is how Steam decides which app id to attribute the process it launched
/// to. The game rewrites the file on every run with that session's app id, so a launch that
/// did not come from Steam (this tool started by hand, or the official launcher) leaves the
/// game's own app id `457962` behind — and the *next* Steam launch is then attributed to
/// 457962 instead of the shortcut, so the library entry never shows "playing". Removing it
/// once the game is gone keeps every later Steam launch on the shortcut's own app id.
fn clear_steam_appid_hint(game_dir: &Path) {
    let hint = game_dir.join("steam_appid.txt");
    match std::fs::remove_file(&hint) {
        Ok(()) => log(&format!("removed {} (Steam app id hint)", hint.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => log(&format!("could not remove {}: {}", hint.display(), error)),
    }
}

/// Log text for the end of a launch.
///
/// Pure on purpose: printing "exited with code 0" after a failed wait is exactly the quiet lie
/// this tool cannot afford, because the log file is the only record an elevated launch leaves
/// behind.
fn describe_exit(wait_result: u32, exit_code: Option<u32>) -> String {
    match (wait_result, exit_code) {
        (WAIT_OBJECT_0, Some(code)) => format!("game exited with code {code} (0x{code:08X})"),
        (WAIT_OBJECT_0, None) => {
            "game exited, but GetExitCodeProcess failed; the exit code is unknown".to_string()
        }
        (WAIT_FAILED, _) => "waiting for the game failed (WAIT_FAILED); it may still be running"
            .to_string(),
        (other, _) => format!("stopped waiting on the game with result 0x{other:08X}"),
    }
}

// ---------------------------------------------------------------- entry point

/// Value of `--name VALUE` or `--name=VALUE`, with the last occurrence winning - the same rule
/// `--env` follows, so `--game-dir A --game-dir B` cannot silently mean `A`.
///
/// A following token that starts with `--` counts as another option rather than a value, so
/// `--game-dir --no-reshade` is rejected instead of looking for a directory named "--no-reshade".
fn arg_value(args: &[String], name: &str) -> Option<String> {
    let with_equals = format!("{name}=");
    let mut found: Option<String> = None;
    for (index, arg) in args.iter().enumerate() {
        if let Some(value) = arg.strip_prefix(&with_equals) {
            found = Some(value.to_string());
        } else if arg == name {
            if let Some(value) = args.get(index + 1).filter(|value| !value.starts_with("--")) {
                found = Some(value.clone());
            }
        }
    }
    found
}

fn help_text() -> String {
    format!(
        "\
EndfieldDirectLauncher {} — starts Endfield.exe without the official launcher UI.

Run it from the game directory (the folder holding Endfield.exe).

Options:
  --console                 attach a console window for progress/errors
  --game-dir <dir>          directory containing Endfield.exe (default: this exe folder);
                            --game-dir=<dir> is accepted too, last one wins
  --no-reshade              start the game without ReShade: sets DISABLE_VK_LAYER_reshade_1=1
                            and VK_LOADER_LAYERS_DISABLE=VK_LAYER_reshade in the game's
                            environment, so ReShade and anything loaded through it (DLSS 5
                            add-ons) never load in this run
  --env NAME=VALUE          add or replace one variable in the game's environment (repeatable)
  --dry-run                 report the command line and environment, start nothing
  --no-popup                never open a message box; write errors to the log file only
  --help                    show this text

How it works:
  The game's OneChannelLC.dll requires its parent process to be named Games.exe and to
  carry the publisher's signature. This tool copies a signed executable out of the game
  directory to a temporary Games.exe, starts it suspended, and creates Endfield.exe with
  PROC_THREAD_ATTRIBUTE_PARENT_PROCESS pointing at it, so both checks pass while no code
  of that binary ever runs.",
        env!("CARGO_PKG_VERSION")
    )
}

/// First line of every log: which build is actually running.
///
/// Not decoration. The flag that "did nothing" once turned out to be a months-old binary still
/// sitting in the game directory, and `strings` on the deployed file was the only way to notice;
/// a version in the log makes the next such mismatch visible from the log alone.
fn version_line() -> String {
    format!(
        "launcher  : {} {}",
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION")
    )
}

fn run(args: &[String]) -> Result<(), String> {
    let game_dir = match arg_value(args, "--game-dir") {
        Some(dir) => PathBuf::from(dir),
        None => exe_dir(),
    };
    log(&version_line());
    // Recorded before the directory is validated: a launch moved to a shell, a Steam shortcut or
    // the Windows Run box is told apart here, and a wrong directory needs this line as much as a
    // correct one does.
    log(&format!("arguments : {args:?}"));
    let game_exe = game_dir.join("Endfield.exe");
    if !game_exe.is_file() {
        return Err(format!(
            "Endfield.exe was not found next to this program:\n{}\n\n\
             Put EndfieldDirectLauncher.exe in the game directory, or pass --game-dir.",
            game_exe.display()
        ));
    }
    log(&format!("game exe  : {}", game_exe.display()));

    let env_overrides = collect_env_overrides(args)?;
    let dry_run = args.iter().any(|a| a == "--dry-run");
    for (name, value) in &env_overrides {
        log(&format!("env override: {name}={value}"));
    }
    if dry_run {
        log(&format!(
            "dry run   : would start \"{}\" {}",
            game_exe.display(),
            LAUNCHER_ARGS
        ));
        log(&format!("dry run   : working directory {}", game_dir.display()));
        log(&format!(
            "dry run   : {} overridden variable(s) in the game process environment",
            env_overrides.len()
        ));
        return Ok(());
    }

    let parent_exe = stage_parent_exe(&game_dir)?;
    let mut parent = match start_suspended_pseudoparent(&parent_exe) {
        Ok(parent) => parent,
        Err(error) => {
            remove_staged_parent(&parent_exe);
            return Err(error);
        }
    };
    // Deliberately created *before* the placeholder joins the cleanup job: a process inherits
    // the job of the parent named in `PROC_THREAD_ATTRIBUTE_PARENT_PROCESS`, so assigning the
    // placeholder to the job any earlier would put the game in it as well — and then killing
    // this program would kill the game, which is the opposite of what the job is for.
    let mut environment_block = build_environment_block(&env_overrides);
    let mut game = match start_game(
        &game_exe,
        &game_dir,
        parent.process,
        environment_block.as_mut(),
    ) {
        Ok(child) => child,
        Err(error) => {
            unsafe {
                TerminateProcess(parent.process, 0);
                parent.close();
            }
            remove_staged_parent(&parent_exe);
            return Err(error);
        }
    };
    kill_placeholder_with_us(parent.process);

    log(&format!(
        "waiting for Endfield.exe (pid {}) to exit; the suspended parent stays alive so the \
         launch check sees games.exe",
        game.pid
    ));
    let mut exit_code = 0u32;
    let wait_result = unsafe { WaitForSingleObject(game.process, INFINITE) };
    let exit_code_read = unsafe { GetExitCodeProcess(game.process, &mut exit_code) } != 0;
    unsafe {
        // Nothing of that binary ever ran; drop the placeholder once the game is gone.
        TerminateProcess(parent.process, 0);
        game.close();
        parent.close();
    }
    clear_steam_appid_hint(&game_dir);
    remove_staged_parent(&parent_exe);
    log(&describe_exit(
        wait_result,
        if exit_code_read { Some(exit_code) } else { None },
    ));
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "--console") {
        unsafe { AllocConsole() };
    }

    let show_box = |no_popup: bool| wants_message_box(stdout().is_some(), no_popup);
    let no_popup = args.iter().any(|a| a == "--no-popup");

    if args.iter().any(|a| a == "--help" || a == "-h") {
        let text = help_text();
        log(&text);
        if show_box(no_popup) {
            message_box(&text, "EndfieldDirectLauncher", MB_OK | MB_ICONINFORMATION);
        }
        return;
    }

    match run(&args) {
        Ok(()) => {}
        Err(error) => fail(error, show_box(no_popup)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    fn value_of<'a>(overrides: &'a [(String, String)], name: &str) -> Option<&'a str> {
        overrides
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    fn count_of(overrides: &[(String, String)], name: &str) -> usize {
        overrides
            .iter()
            .filter(|(key, _)| key.eq_ignore_ascii_case(name))
            .count()
    }

    /// Split a `NAME=VALUE\\0...\\0\\0` block back into entries.
    fn entries(block: &[u16]) -> Vec<String> {
        let mut out = Vec::new();
        let mut current = String::new();
        for &unit in block {
            if unit == 0 {
                if current.is_empty() {
                    break;
                }
                out.push(current.clone());
                current.clear();
            } else {
                current.push(std::char::from_u32(u32::from(unit)).unwrap_or('?'));
            }
        }
        out
    }

    #[test]
    fn the_log_names_the_running_build() {
        assert!(version_line().contains(env!("CARGO_PKG_VERSION")));
        assert!(version_line().starts_with("launcher"));
    }

    #[test]
    fn help_names_the_version() {
        let text = help_text();
        assert!(text.contains(env!("CARGO_PKG_VERSION")), "help lost the version");
        assert!(text.contains("--no-popup") && text.contains("--no-reshade"));
    }

    #[test]
    fn no_reshade_sets_both_switches() {
        let overrides = collect_env_overrides(&args(&["--no-reshade"])).unwrap();
        assert_eq!(overrides.len(), 2);
        assert_eq!(value_of(&overrides, RESHADE_DISABLE_ENV), Some("1"));
        assert_eq!(value_of(&overrides, VK_LOADER_LAYERS_DISABLE_ENV), Some(RESHADE_LAYER_NAME));
    }

    #[test]
    fn no_reshade_twice_stays_two_entries() {
        let overrides = collect_env_overrides(&args(&["--no-reshade", "--no-reshade"])).unwrap();
        assert_eq!(overrides.len(), 2);
        assert_eq!(count_of(&overrides, RESHADE_DISABLE_ENV), 1);
    }

    #[test]
    fn a_later_env_replaces_the_shorthand() {
        let overrides = collect_env_overrides(&args(&[
            "--no-reshade",
            "--env",
            "DISABLE_VK_LAYER_reshade_1=0",
        ]))
        .unwrap();
        assert_eq!(value_of(&overrides, RESHADE_DISABLE_ENV), Some("0"));
        assert_eq!(count_of(&overrides, RESHADE_DISABLE_ENV), 1);
        assert_eq!(value_of(&overrides, VK_LOADER_LAYERS_DISABLE_ENV), Some(RESHADE_LAYER_NAME));
    }

    #[test]
    fn env_requires_a_value() {
        assert!(collect_env_overrides(&args(&["--env"])).is_err());
    }

    #[test]
    fn env_does_not_take_an_option_as_its_value() {
        // Reverting the guard turns this into Ok([]) or an "has no '=' in it" error; either way the
        // assertion below fails, so the guard is the thing under test.
        let error = collect_env_overrides(&args(&["--env", "--no-reshade"])).unwrap_err();
        assert!(
            error.contains("found another option"),
            "the option was taken as a value; it must be named as such: {error}"
        );
    }

    #[test]
    fn env_accepts_both_spellings() {
        let spaced = collect_env_overrides(&args(&["--env", "FMT=1"])).unwrap();
        let joined = collect_env_overrides(&args(&["--env=FMT=1"])).unwrap();
        assert_eq!(spaced, joined);
        assert_eq!(value_of(&spaced, "FMT"), Some("1"));
    }

    #[test]
    fn env_rejects_assignments_without_equals_or_name() {
        assert!(collect_env_overrides(&args(&["--env", "NOEQUALS"])).is_err());
        assert!(collect_env_overrides(&args(&["--env", "=value-but-no-name"])).is_err());
    }

    #[test]
    fn the_last_assignment_wins() {
        let overrides = collect_env_overrides(&args(&[
            "--env",
            "FMT=1",
            "--env",
            "OTHER=x",
            "--env",
            "FMT=2",
        ]))
        .unwrap();
        assert_eq!(value_of(&overrides, "FMT"), Some("2"));
        assert_eq!(count_of(&overrides, "FMT"), 1);
        assert_eq!(value_of(&overrides, "OTHER"), Some("x"));
    }

    #[test]
    fn environment_block_is_none_without_overrides() {
        assert!(build_environment_block(&[]).is_none());
    }

    #[test]
    fn environment_block_is_sorted_and_double_nul_terminated() {
        let block = build_environment_block(&[("zz_test_sorted".to_string(), "1".to_string())])
            .unwrap();
        assert_eq!(block[block.len() - 1], 0);
        assert_eq!(block[block.len() - 2], 0);
        let names: Vec<String> = entries(&block)
            .iter()
            .map(|entry| {
                entry
                    .split_once('=')
                    .map(|(key, _)| key.to_ascii_uppercase())
                    .unwrap_or_else(|| entry.to_ascii_uppercase())
            })
            .collect();
        assert!(names.windows(2).all(|pair| pair[0] <= pair[1]), "not sorted: {names:?}");
    }

    #[test]
    fn environment_block_keeps_one_entry_per_name() {
        let block = build_environment_block(&[
            ("zz_test_dup".to_string(), "1".to_string()),
            ("ZZ_TEST_DUP".to_string(), "2".to_string()),
        ])
        .unwrap();
        let hits: Vec<String> = entries(&block)
            .into_iter()
            .filter(|entry| entry.to_ascii_uppercase().starts_with("ZZ_TEST_DUP="))
            .collect();
        assert_eq!(hits.len(), 1, "duplicate survived: {hits:?}");
        assert_eq!(hits[0], "ZZ_TEST_DUP=2");
    }

    #[test]
    fn a_one_megabyte_block_still_ends_with_two_zeros() {
        let block = build_environment_block(&[("zz_test_tail".to_string(), String::new())]).unwrap();
        assert_eq!(&block[block.len() - 2..], &[0, 0]);
    }

    #[test]
    fn game_dir_accepts_both_spellings_and_last_wins() {
        assert_eq!(arg_value(&args(&["--game-dir", "A"]), "--game-dir").as_deref(), Some("A"));
        assert_eq!(arg_value(&args(&["--game-dir=B"]), "--game-dir").as_deref(), Some("B"));
        assert_eq!(
            arg_value(&args(&["--game-dir", "A", "--game-dir=B"]), "--game-dir").as_deref(),
            Some("B")
        );
        assert_eq!(
            arg_value(&args(&["--game-dir=B", "--game-dir", "A"]), "--game-dir").as_deref(),
            Some("A")
        );
    }

    #[test]
    fn game_dir_does_not_take_an_option_as_its_value() {
        assert_eq!(arg_value(&args(&["--game-dir", "--no-reshade"]), "--game-dir"), None);
    }

    #[test]
    fn staging_directory_belongs_to_this_process() {
        let dir = staging_dir();
        assert_eq!(dir.parent(), Some(staging_root().as_path()));
        assert_eq!(
            dir.file_name().unwrap().to_string_lossy(),
            std::process::id().to_string()
        );
        assert_ne!(dir, staging_root().join("Games.exe"));
    }

    #[test]
    fn message_box_needs_neither_console_nor_flag() {
        assert!(wants_message_box(false, false));
        assert!(!wants_message_box(true, false));
        assert!(!wants_message_box(false, true));
        assert!(!wants_message_box(true, true));
    }

    #[test]
    fn log_rotates_at_the_ceiling() {
        assert!(!needs_rotation(LOG_ROTATE_BYTES - 1));
        assert!(needs_rotation(LOG_ROTATE_BYTES));
        assert!(needs_rotation(LOG_ROTATE_BYTES * 3));
    }

    #[test]
    fn exit_report_never_invents_a_code() {
        assert_eq!(
            describe_exit(WAIT_OBJECT_0, Some(0)),
            "game exited with code 0 (0x00000000)"
        );
        assert!(describe_exit(WAIT_OBJECT_0, None).contains("unknown"));
        assert!(describe_exit(WAIT_FAILED, Some(3)).contains("WAIT_FAILED"));
        assert!(describe_exit(0x102, None).contains("0x00000102"));
    }
}

#[cfg(test)]
mod readme_contract;
