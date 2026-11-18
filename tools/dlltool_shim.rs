// A `dlltool.exe` that rustc can actually use on a windows-gnu host.
//
// ── WHY THIS EXISTS ──────────────────────────────────────────────────────────
// Cross-compiling to android from a windows-gnu host still builds HOST build-deps
// (`getrandom` -> `bcryptprimitives`, `windows-sys` -> `kernel32`), and those declare
// their imports with `kind = "raw-dylib"`. rustc materialises those into import libs
// by shelling out to `dlltool`, and it picks the GNU or the LLVM flag set from the
// tool's FILE NAME. The name it looks for is plain `dlltool`, so a renamed
// llvm-dlltool is still handed the GNU-only set. Measured invocation:
//
//   dlltool -d <def> -D <dll> -l <lib> -m i386:x86-64 -f --64 \
//           --no-leading-underscore --temp-prefix <name>
//
// `--temp-prefix` is fatal: llvm-dlltool answers an unknown argument by printing its
// usage and exiting 1, so no import lib is written and the build dies with the
// misleading `failed to add native library ...dll_imports.lib: file not found`.
// (`--no-leading-underscore` only warns.)
//
// The GNU dlltool that rustup ships in `self-contained` is not an option either: it
// shells out to `as`, which the toolchain does not ship.
//
// So this filters the command line down to the flags llvm-dlltool documents
// (`-D -d -f -k -l -m -S`) and forwards. Built by `build_android.ps1`; the real tool
// comes from `WIE_REAL_DLLTOOL`.
use std::{env, process::Command};

/// Flags llvm-dlltool documents that take a separate value.
const WITH_VALUE: [&str; 6] = ["-D", "-d", "-f", "-l", "-m", "-S"];
/// Flags llvm-dlltool documents that take none.
const FLAGS: [&str; 1] = ["-k"];

fn main() {
    let real = env::var("WIE_REAL_DLLTOOL").unwrap_or_else(|_| "llvm-dlltool.exe".into());
    let argv: Vec<String> = env::args().skip(1).collect();
    let mut out: Vec<String> = Vec::with_capacity(argv.len());
    let mut i = 0;
    while i < argv.len() {
        let a = &argv[i];
        if WITH_VALUE.contains(&a.as_str()) {
            out.push(a.clone());
            if let Some(v) = argv.get(i + 1) {
                out.push(v.clone());
            }
            i += 2;
        } else if FLAGS.contains(&a.as_str()) {
            out.push(a.clone());
            i += 1;
        } else if a.starts_with('-') {
            // Unknown to llvm-dlltool. `--temp-prefix` carries a value, which has to be
            // dropped with it or it would be read as a positional input file.
            i += if a == "--temp-prefix" { 2 } else { 1 };
        } else {
            out.push(a.clone()); // positional input file
            i += 1;
        }
    }
    match Command::new(&real).args(&out).status() {
        Ok(st) => std::process::exit(st.code().unwrap_or(1)),
        Err(e) => {
            eprintln!("dlltool shim: cannot run {real}: {e}");
            std::process::exit(1);
        }
    }
}
