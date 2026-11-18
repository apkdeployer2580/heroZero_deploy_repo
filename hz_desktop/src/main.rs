//! hz_desktop — 영웅서기 Zero on the PC. The program is [`desktop`]; this file only chooses it:
//! ⛔ the host crate's window and worker (`hz_android::app`) exist on Android and under the MSVC
//! toolchain only — on the windows-gnu host (the probe's toolchain) the audio stack dies before
//! `main` (2026-10-29), so a gnu build of this binary says so and exits.

#[cfg(target_env = "msvc")]
mod desktop;

#[cfg(target_env = "msvc")]
fn main() {
    desktop::main();
}

#[cfg(not(target_env = "msvc"))]
fn main() {
    eprintln!("herozero: this binary must be built with the MSVC toolchain (build_desktop.ps1): cargo +stable-x86_64-pc-windows-msvc build -p hz_desktop");
    std::process::exit(2);
}
