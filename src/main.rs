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

#![windows_subsystem = "windows"]
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
const EXTENDED_STARTUPINFO_PRESENT: u32 = 0x0008_0000;
const PROC_THREAD_ATTRIBUTE_PARENT_PROCESS: usize = 0x0002_0000;
const INFINITE: u32 = 0xFFFF_FFFF;
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
    let Ok(mut file) = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("EndfieldDirectLauncher.log"))
    else {
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

fn fail(text: String) -> ! {
    log(&format!("ERROR: {}", text));
    message_box(&text, "EndfieldDirectLauncher", MB_OK | MB_ICONERROR);
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

    let staging_dir = std::env::temp_dir().join("EndfieldDirectLauncher");
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

/// Drop the staged copy once the placeholder is gone. A leftover would be harmless (the next
/// run overwrites it) but pointless. Windows releases a terminated process' image section a
/// moment after the fact, so a few retries are needed before the file will actually go.
fn remove_staged_parent(path: &Path) {
    for attempt in 0..10 {
        match fs::remove_file(path) {
            Ok(()) => {
                log(&format!("removed {}", path.display()));
                return;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
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
fn start_game(game_exe: &Path, game_dir: &Path, parent: HANDLE) -> Result<Child, String> {
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

    let mut info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    let ok = unsafe {
        CreateProcessW(
            application.as_ptr(),
            command_line.as_mut_ptr(),
            null_mut(),
            null_mut(),
            0,
            EXTENDED_STARTUPINFO_PRESENT,
            null_mut(),
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

// ---------------------------------------------------------------- entry point

fn arg_value(args: &[String], name: &str) -> Option<String> {
    let index = args.iter().position(|a| a == name)?;
    args.get(index + 1).cloned()
}

fn help_text() -> String {
    "\
EndfieldDirectLauncher — starts Endfield.exe without the official launcher UI.

Run it from the game directory (the folder holding Endfield.exe).

Options:
  --console                 attach a console window for progress/errors
  --game-dir <dir>          directory containing Endfield.exe (default: this exe's folder)
  --help                    show this text

How it works:
  The game's OneChannelLC.dll requires its parent process to be named Games.exe and to
  carry the publisher's signature. This tool copies a signed executable out of the game
  directory to a temporary Games.exe, starts it suspended, and creates Endfield.exe with
  PROC_THREAD_ATTRIBUTE_PARENT_PROCESS pointing at it, so both checks pass while no code
  of that binary ever runs."
        .to_string()
}

fn run(args: &[String]) -> Result<(), String> {
    let game_dir = match arg_value(args, "--game-dir") {
        Some(dir) => PathBuf::from(dir),
        None => exe_dir(),
    };
    let game_exe = game_dir.join("Endfield.exe");
    if !game_exe.is_file() {
        return Err(format!(
            "Endfield.exe was not found next to this program:\n{}\n\n\
             Put EndfieldDirectLauncher.exe in the game directory, or pass --game-dir.",
            game_exe.display()
        ));
    }
    log(&format!("game exe  : {}", game_exe.display()));

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
    let mut game = match start_game(&game_exe, &game_dir, parent.process) {
        Ok(child) => child,
        Err(error) => {
            unsafe { TerminateProcess(parent.process, 0) };
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
    unsafe {
        WaitForSingleObject(game.process, INFINITE);
        // Nothing of that binary ever ran; drop the placeholder once the game is gone.
        TerminateProcess(parent.process, 0);
        game.close();
        parent.close();
    }
    clear_steam_appid_hint(&game_dir);
    remove_staged_parent(&parent_exe);
    log("game exited");
    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "--console") {
        unsafe { AllocConsole() };
    }

    if args.iter().any(|a| a == "--help" || a == "-h") {
        let text = help_text();
        log(&text);
        if stdout().is_none() {
            message_box(&text, "EndfieldDirectLauncher", MB_OK | MB_ICONINFORMATION);
        }
        return;
    }

    match run(&args) {
        Ok(()) => {}
        Err(error) => fail(error),
    }
}
