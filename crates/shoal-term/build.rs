//! Compile-time parts of the core identity (M1 spec §5.3).
//!
//! The bindings revision comes from `Cargo.lock`. The Ghostty commit is not in
//! the lockfile: libghostty-vt-sys fetches it at a commit fixed in its own build
//! script. It is recorded here next to the bindings revision it belongs to, so a
//! bindings pin change fails this build until both are updated together (the
//! T07 pin-upgrade rule). `tests/core.rs` checks the pair against the sources.

use std::{env, fs, path::Path};

const BINDINGS_REV: &str = "9ede35b8ea94a55ed3608408d67e102883129f1e";
const GHOSTTY_REV: &str = "521cf2c5d30bd48028ca61546c64e7913764073d";

fn main() {
    let manifest = env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR");
    let lock = Path::new(&manifest).join("../../Cargo.lock");
    println!("cargo:rerun-if-changed={}", lock.display());
    println!("cargo:rerun-if-env-changed=LIBGHOSTTY_VT_SYS_CPU");
    let text = fs::read_to_string(&lock)
        .unwrap_or_else(|error| panic!("reading {}: {error}", lock.display()));
    let locked = bindings_rev(&text).unwrap_or_else(|| {
        panic!("Cargo.lock has no git revision for libghostty-vt");
    });
    assert_eq!(
        locked, BINDINGS_REV,
        "libghostty-vt moved to {locked}: update BINDINGS_REV and GHOSTTY_REV in \
         crates/shoal-term/build.rs to the new pin, then rerun the snapshot tests"
    );
    // The same default libghostty-vt-sys uses when the variable is unset.
    let cpu = env::var("LIBGHOSTTY_VT_SYS_CPU").unwrap_or_else(|_| "baseline".into());
    println!("cargo:rustc-env=SHOAL_BINDINGS_REV={locked}");
    println!("cargo:rustc-env=SHOAL_GHOSTTY_REV={GHOSTTY_REV}");
    println!("cargo:rustc-env=SHOAL_GHOSTTY_CPU={cpu}");
}

/// The 40-hex revision after `#` in libghostty-vt's lockfile `source`.
fn bindings_rev(lock: &str) -> Option<&str> {
    let package = lock
        .split("[[package]]")
        .find(|package| package.contains("\nname = \"libghostty-vt\"\n"))?;
    let source = package
        .lines()
        .find_map(|line| line.strip_prefix("source = \""))?;
    let rev = source.trim_end_matches('"').rsplit_once('#')?.1;
    (rev.len() == 40 && rev.bytes().all(|byte| byte.is_ascii_hexdigit())).then_some(rev)
}
