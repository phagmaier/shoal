use std::{fs, path::Path, process::Command};

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

/// The `rev` after `#` in a lockfile package's git source.
fn locked_rev(lock: &str, name: &str) -> String {
    let package = lock
        .split("[[package]]")
        .find(|package| package.contains(&format!("\nname = \"{name}\"\n")))
        .unwrap();
    let source = package
        .lines()
        .find_map(|line| line.strip_prefix("source = \""))
        .unwrap();
    source
        .trim_end_matches('"')
        .rsplit_once('#')
        .unwrap()
        .1
        .into()
}

/// The Ghostty commit libghostty-vt-sys's build script actually fetches.
fn sys_ghostty_commit() -> String {
    let output = Command::new(env!("CARGO"))
        .args(["metadata", "--locked", "--offline", "--format-version", "1"])
        .current_dir(ROOT)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let manifest = metadata["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|package| package["name"] == "libghostty-vt-sys")
        .and_then(|package| package["manifest_path"].as_str())
        .unwrap();
    let build = fs::read_to_string(Path::new(manifest).with_file_name("build.rs")).unwrap();
    let line = build
        .lines()
        .find(|line| line.starts_with("const GHOSTTY_COMMIT: &str = \""))
        .unwrap();
    line.split('"').nth(1).unwrap().into()
}

#[test]
fn core_id_names_the_pinned_revisions_and_linked_build_settings() {
    let core = shoal_term::core_id();
    let fields: Vec<(&str, &str)> = core
        .split(';')
        .map(|field| field.split_once('=').unwrap())
        .collect();
    let keys: Vec<_> = fields.iter().map(|(key, _)| *key).collect();
    assert_eq!(
        keys,
        [
            "ghostty",
            "libghostty-rs",
            "version",
            "optimize",
            "cpu",
            "simd",
            "kitty_graphics",
            "tmux"
        ]
    );
    let field = |key| fields.iter().find(|(name, _)| *name == key).unwrap().1;

    let lock = fs::read_to_string(Path::new(ROOT).join("Cargo.lock")).unwrap();
    assert_eq!(field("libghostty-rs"), locked_rev(&lock, "libghostty-vt"));
    assert_eq!(
        field("libghostty-rs"),
        locked_rev(&lock, "libghostty-vt-sys")
    );
    assert_eq!(field("ghostty"), sys_ghostty_commit());
    // The read-only headers agents consult come from the same Ghostty commit.
    let fetch = fs::read_to_string(Path::new(ROOT).join("third_party/fetch.sh")).unwrap();
    assert!(fetch.contains(&format!("ghostty_rev={}\n", field("ghostty"))));
    for key in ["ghostty", "libghostty-rs"] {
        let rev = field(key);
        assert!(
            rev.len() == 40 && rev.bytes().all(|b| b.is_ascii_hexdigit()),
            "{rev}"
        );
    }
    // The workspace builds libghostty with ReleaseFast for the baseline CPU.
    assert_eq!(field("optimize"), "ReleaseFast");
    assert_eq!(field("cpu"), "baseline");
    for key in ["simd", "kitty_graphics", "tmux"] {
        assert!(
            ["true", "false"].contains(&field(key)),
            "{key}={}",
            field(key)
        );
    }
    assert!(
        !field("version").is_empty() && !core.contains("error("),
        "{core}"
    );
    // Computed once: callers compare the same string.
    assert!(std::ptr::eq(core, shoal_term::core_id()));
}
