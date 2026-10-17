//! EndfieldDirectLauncher — start `Endfield.exe` directly, bypassing the official launcher UI.
//!
//! Why a plain `Start-Process Endfield.exe` does not work:
//! the game ships `OneChannelLC.dll` ("LC" = launch check). At SDK init it takes a
//! toolhelp snapshot, walks to its own parent PID, resolves the parent's image path with
//! `QueryFullProcessImageNameW` and compares the file *name* against a configured list.
//! Only `Games.exe` (the launcher's game-launch helper) is accepted; anything else
//! (explorer.exe, steam.exe, maaend.exe) yields
//! `{"code":1,"msg":{"errorCode":10100,"filePath":"..."}}` followed by
//! `PC launch auth extraInfo failed` and a "start the official launcher" prompt.
//!
//! The bypass is therefore not a patch but a parenting trick: `Games.exe` is created
//! suspended and then named as the new process' parent via
//! `PROC_THREAD_ATTRIBUTE_PARENT_PROCESS`, so the kernel reports `games.exe` as the
//! parent while nothing of the launcher ever runs.
//!
//! Elevation is declared in the binary manifest (`build.rs` passes
//! `/MANIFESTUAC:level='requireAdministrator'`) because `Endfield.exe` is itself
//! `requireAdministrator`. Windows therefore elevates this program once, at startup, and it
//! stays a single process for the whole session. That is not merely tidy: a Steam non-Steam
//! shortcut stops reporting "playing" the moment the process Steam launched exits, and
//! Steam's overlay can only follow into the game if the whole chain belongs to the process
//! Steam injected it into.
//!
//! No dependencies; raw `kernel32` / `advapi32` / `user32` FFI only.

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

/// CN region identifiers, taken verbatim from MaaEnd's
/// `agent/go-service/pretask/launchgamepc/launchgamepc.go` (`regionCN`).
const CN_UNINSTALL_KEY: &str = "33a0a6296a20400d503c59ac0fd6341e";
const CN_PROTOCOL_KEY: &str = "hypergryphlauncher-abYeZZ16BPluCFyT";
const UNINSTALL_PREFIX: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\";

// ---------------------------------------------------------------- Win32 FFI

type HANDLE = *mut c_void;
type BOOL = i32;
type HKEY = usize;

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

const HKEY_CLASSES_ROOT: HKEY = 0x8000_0000;
const HKEY_CURRENT_USER: HKEY = 0x8000_0001;
const HKEY_LOCAL_MACHINE: HKEY = 0x8000_0002;
const KEY_QUERY_VALUE: u32 = 0x0001;
const KEY_WOW64_32KEY: u32 = 0x0200;
const KEY_WOW64_64KEY: u32 = 0x0100;
const REG_SZ: u32 = 1;

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

#[link(name = "advapi32")]
extern "system" {
    fn RegOpenKeyExW(
        hKey: HKEY,
        lpSubKey: *const u16,
        ulOptions: u32,
        samDesired: u32,
        phkResult: *mut HKEY,
    ) -> i32;
    fn RegQueryValueExW(
        hKey: HKEY,
        lpValueName: *const u16,
        lpReserved: *mut u32,
        lpType: *mut u32,
        lpData: *mut u8,
        lpcbData: *mut u32,
    ) -> i32;
    fn RegCloseKey(hKey: HKEY) -> i32;
}

// ---------------------------------------------------------------- small helpers

fn wide(s: &str) -> Vec<u16> {
    OsString::from(s).encode_wide().chain(Some(0)).collect()
}

fn wide_path(p: &Path) -> Vec<u16> {
    p.as_os_str().encode_wide().chain(Some(0)).collect()
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

// ---------------------------------------------------------------- locating the launcher

/// One string value out of the registry, or `None`.
fn reg_string(root: HKEY, subkey: &str, value: &str, extra_access: u32) -> Option<String> {
    let mut hkey: HKEY = 0;
    if unsafe {
        RegOpenKeyExW(
            root,
            wide(subkey).as_ptr(),
            0,
            KEY_QUERY_VALUE | extra_access,
            &mut hkey,
        )
    } != 0
    {
        return None;
    }
    let name = wide(value);
    let mut kind = 0u32;
    let mut size = 0u32;
    let status = unsafe {
        RegQueryValueExW(hkey, name.as_ptr(), null_mut(), &mut kind, null_mut(), &mut size)
    };
    if status != 0 || size == 0 || kind != REG_SZ {
        unsafe { RegCloseKey(hkey) };
        return None;
    }
    let mut buf = vec![0u8; size as usize + 2];
    let mut filled = size;
    let status = unsafe {
        RegQueryValueExW(hkey, name.as_ptr(), null_mut(), &mut kind, buf.as_mut_ptr(), &mut filled)
    };
    unsafe { RegCloseKey(hkey) };
    if status != 0 {
        return None;
    }
    let units: Vec<u16> = buf[..filled as usize]
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes(*c))
        .collect();
    let end = units.iter().position(|&c| c == 0).unwrap_or(units.len());
    Some(String::from_utf16_lossy(&units[..end]))
}

/// Launcher install directories, most trustworthy first. Same lookup order as MaaEnd's
/// `FindLauncherInstallDir` / `FindLauncherInstallDirByProtocol`.
fn find_launcher_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let subkey = format!("{}{}", UNINSTALL_PREFIX, CN_UNINSTALL_KEY);
    // The launcher is a 32-bit installer, so its uninstall entry lives in WOW6432Node
    // (KEY_WOW64_32KEY); the other two views are fallbacks.
    for (root, access) in [
        (HKEY_LOCAL_MACHINE, KEY_WOW64_32KEY),
        (HKEY_LOCAL_MACHINE, KEY_WOW64_64KEY),
        (HKEY_CURRENT_USER, 0),
    ] {
        if let Some(dir) = reg_string(root, &subkey, "InstallLocation", access) {
            let dir = PathBuf::from(dir.trim().trim_matches('"'));
            // A stale entry can survive a moved/deleted launcher.
            if dir.join("Launcher.exe").is_file() {
                dirs.push(dir);
            }
        }
    }

    let protocol = format!(r"{}\shell\open\command", CN_PROTOCOL_KEY);
    if let Some(command) = reg_string(HKEY_CLASSES_ROOT, &protocol, "", 0) {
        if let Some(exe) = first_token_path(&command) {
            let looks_right = exe
                .file_name()
                .map(|n| n.eq_ignore_ascii_case("Launcher.exe"))
                .unwrap_or(false);
            if looks_right && exe.is_file() {
                if let Some(parent) = exe.parent() {
                    dirs.push(parent.to_path_buf());
                }
            }
        }
    }
    dirs
}

/// Fallback for when the registry says nothing: `<root>\games\<Game Dir>` -> `<root>`.
fn launcher_dirs_from_game_dir(game_dir: &Path) -> Vec<PathBuf> {
    match game_dir.parent().and_then(Path::parent) {
        Some(root) => vec![root.to_path_buf()],
        None => Vec::new(),
    }
}

/// `"D:\x\Launcher.exe" "%1"` -> `D:\x\Launcher.exe`
fn first_token_path(command: &str) -> Option<PathBuf> {
    let trimmed = command.trim();
    if let Some(rest) = trimmed.strip_prefix('"') {
        return Some(PathBuf::from(&rest[..rest.find('"')?]));
    }
    let end = trimmed.find(' ').unwrap_or(trimmed.len());
    Some(PathBuf::from(&trimmed[..end]))
}

/// `<Games.exe>` either at the launcher root or one version-directory below it
/// (`<root>\1.6.0\Games.exe`); the newest build wins.
fn find_games_exe(launcher_dir: &Path) -> Option<PathBuf> {
    let at_root = launcher_dir.join("Games.exe");
    if at_root.is_file() {
        return Some(at_root);
    }
    let mut newest: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in fs::read_dir(launcher_dir).ok()?.flatten() {
        let candidate = entry.path().join("Games.exe");
        if !candidate.is_file() {
            continue;
        }
        let stamp = entry
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::UNIX_EPOCH);
        if newest.as_ref().map(|(best, _)| stamp > *best).unwrap_or(true) {
            newest = Some((stamp, candidate));
        }
    }
    newest.map(|(_, path)| path)
}

/// Resolve the launcher's `Games.exe`, the one file the launch check accepts as parent.
fn lookup_games_exe(args: &[String], game_dir: &Path) -> Result<PathBuf, String> {
    let explicit = arg_value(args, "--launcher-dir").map(PathBuf::from);
    let candidates: Vec<PathBuf> = match explicit {
        Some(ref dir) => vec![dir.clone()],
        None => find_launcher_dirs()
            .into_iter()
            .chain(launcher_dirs_from_game_dir(game_dir))
            .collect(),
    };

    for dir in &candidates {
        if let Some(exe) = find_games_exe(dir) {
            log(&format!("launcher dir: {}", dir.display()));
            return Ok(exe);
        }
    }

    Err(match explicit {
        Some(dir) => format!("No Games.exe inside:\n{}", dir.display()),
        None => "Could not locate the official launcher, so Games.exe is unavailable.\n\n\
                 Pass it explicitly, e.g. --launcher-dir \"F:\\Hypergryph Launcher\"."
            .to_string(),
    })
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

/// Start `Games.exe` suspended: it must exist as a process (so it can be named as parent)
/// but must never run, otherwise the launcher would do its own thing.
fn start_suspended_pseudoparent(games_exe: &Path) -> Result<Child, String> {
    let application = wide_path(games_exe);
    let mut command_line = wide(&format!("\"{}\"", games_exe.display()));
    let cwd = games_exe.parent().map(wide_path).unwrap_or_else(|| wide("."));

    let mut startup: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
    startup.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
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
    let mut command_line = wide(&format!("\"{}\" {}", game_exe.display(), LAUNCHER_ARGS));
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
  --launcher-dir <dir>      official launcher install directory, e.g. \"F:\\Hypergryph Launcher\"
  --help                    show this text

How it works:
  The game's OneChannelLC.dll requires its parent process to be named Games.exe.
  This tool starts the launcher's Games.exe suspended and creates Endfield.exe
  with PROC_THREAD_ATTRIBUTE_PARENT_PROCESS pointing at it, so the check passes
  while no launcher code ever executes."
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

    let games_exe = lookup_games_exe(args, &game_dir)?;
    log(&format!("parent exe: {}", games_exe.display()));

    let mut parent = start_suspended_pseudoparent(&games_exe)?;
    let mut game = match start_game(&game_exe, &game_dir, parent.process) {
        Ok(child) => child,
        Err(error) => {
            unsafe { TerminateProcess(parent.process, 0) };
            return Err(error);
        }
    };

    log(&format!(
        "waiting for Endfield.exe (pid {}) to exit; the suspended parent stays alive so the \
         launch check sees games.exe",
        game.pid
    ));
    unsafe {
        WaitForSingleObject(game.process, INFINITE);
        // Nothing of the launcher ever ran; drop the placeholder once the game is gone.
        TerminateProcess(parent.process, 0);
        game.close();
        parent.close();
    }
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
