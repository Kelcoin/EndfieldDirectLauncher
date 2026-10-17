//! Build-time resources for the executable: the `requireAdministrator` level and the icon.
//!
//! The level matters because `Endfield.exe` itself declares it, so the game always runs at
//! High integrity and this program cannot start it from a standard-user token. Declaring the
//! same requirement makes Windows elevate us once, up front, so we stay a single process for
//! the whole session -- which is what keeps a Steam non-Steam shortcut showing as "running"
//! and keeps the process chain inside the process Steam injected its overlay into.
//!
//! Cargo has no field for either resource and a crate would be a dependency for two linker
//! inputs, so both go straight to `link.exe`. Passing a manifest snippet instead fails with
//! `manifest authoring error c1010001` because `link.exe` generates its own UAC fragment.

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    // Both switches are needed: this toolchain emits no manifest resource at all unless
    // embedding is asked for explicitly.
    println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg=/MANIFESTUAC:level='requireAdministrator'");
    embed_icon();
}

/// Compiles `assets/app.ico` into a `.res` and hands that to `link.exe`.
///
/// The icon is the game's own, pulled out of `Endfield.exe` with `assets/extract-icon.py`;
/// only the `.ico` lives in the repo, the `.res` is regenerated on every build.
fn embed_icon() {
    println!("cargo:rerun-if-changed=assets/app.ico");
    let rc = find_rc().unwrap_or_else(|| {
        panic!(
            "rc.exe (the Windows SDK resource compiler) was not found.\n\
             Install the Windows SDK, or point the RC environment variable at rc.exe."
        )
    });

    let out = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is always set by cargo"));
    // rc.exe reads escapes out of quoted file names, so a path full of backslashes gets
    // mangled (`\a` swallows the next character). Copy the icon next to the script and use
    // bare names with the working directory set to that same folder -- nothing left to parse.
    std::fs::copy("assets/app.ico", out.join("app.ico")).expect("assets/app.ico must exist");
    std::fs::write(out.join("icon.rc"), "1 ICON \"app.ico\"\n").expect("writing icon.rc");

    let res = out.join("icon.res");
    let status = Command::new(&rc)
        .current_dir(&out)
        .args(["/nologo", "/fo", "icon.res", "icon.rc"])
        .status()
        .unwrap_or_else(|e| panic!("could not run {}: {e}", rc.display()));
    assert!(status.success(), "{} failed with {status}", rc.display());

    println!("cargo:rustc-link-arg={}", res.display());
}

/// `RC` first (the same escape hatch other Windows builds use), then the Windows SDK kits
/// newest-first, then whatever `rc.exe` is on `PATH`.
fn find_rc() -> Option<PathBuf> {
    if let Some(explicit) = env::var_os("RC") {
        let explicit = PathBuf::from(explicit);
        if explicit.is_file() {
            return Some(explicit);
        }
    }

    let kits = env::var_os("ProgramFiles(x86)")
        .or_else(|| env::var_os("ProgramFiles"))
        .map(PathBuf::from)?
        .join("Windows Kits")
        .join("10")
        .join("bin");
    let mut versions: Vec<PathBuf> = std::fs::read_dir(kits)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    versions.sort();
    for version in versions.into_iter().rev() {
        let exe = version.join("x64").join("rc.exe");
        if exe.is_file() {
            return Some(exe);
        }
    }

    Command::new("rc.exe")
        .arg("/?")
        .output()
        .ok()
        .map(|_| PathBuf::from("rc.exe"))
}
