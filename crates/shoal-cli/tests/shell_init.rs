//! Manual integration, including a relocated installation with quoted paths.
use std::{
    fs,
    io::Write,
    process::{Command, Stdio},
};

#[test]
fn manual_init_loads_once_and_off_is_inert() {
    for shell in ["bash", "zsh"] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("prefix with ' quotes");
        let bin = root.join("bin");
        fs::create_dir_all(&bin).unwrap();
        let cli = bin.join("shoal");
        fs::copy(env!("CARGO_BIN_EXE_shoal"), &cli).unwrap();
        let assets = root.join("share/shoal/shell").join(shell);
        fs::create_dir_all(&assets).unwrap();
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../share/shoal/shell")
            .join(shell)
            .join(format!("shoal.{shell}"));
        fs::copy(source, assets.join(format!("shoal.{shell}"))).unwrap();
        let config = temp.path().join("config/shoal");
        fs::create_dir_all(&config).unwrap();
        fs::write(
            config.join("config.toml"),
            "[shell]\nintegration='manual'\n[prompt]\nmode='shell'\n",
        )
        .unwrap();
        // No rc files are read, so only shell-init can install the hooks.
        let script = format!(
            "eval \"$(\"$CLI\" shell-init {shell})\"; eval \"$(\"$CLI\" shell-init {shell})\"\n(exit 7)\nprintf 'manual:%s\\n' ready\nexit\n"
        );
        let mut child = Command::new("timeout")
            .args([
                "5",
                shell,
                if shell == "bash" { "--norc" } else { "-f" },
                "-i",
            ])
            .env_clear()
            .env("HOME", temp.path())
            .env("XDG_CONFIG_HOME", config.parent().unwrap())
            .env("PATH", "/usr/bin:/bin")
            .env("CLI", &cli)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(script.as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success(), "{shell}: {output:?}");
        let text = String::from_utf8_lossy(&output.stdout).replace("\x1b\\", "\x07");
        assert!(text.contains("manual:ready"), "{shell}: {output:?}");
        assert!(text.contains("\x1b]133;D;7\x07"), "{shell}: {output:?}");
        let prompts = String::from_utf8_lossy(&output.stderr).replace("\x1b\\", "\x07");
        assert!(
            text.contains("\x1b]133;C\x07") || prompts.contains("\x1b]133;C\x07"),
            "{shell}: {output:?}"
        );
        assert!(
            prompts.contains("\x1b]133;A\x07") && prompts.contains("\x1b]133;B\x07"),
            "{shell}: {output:?}"
        );
        assert_eq!(
            text.matches("\x1b]133;D;7\x07").count(),
            1,
            "{shell}: {output:?}"
        );
        fs::write(config.join("config.toml"), "[shell]\nintegration='off'\n").unwrap();
        let output = Command::new(&cli)
            .args(["shell-init", shell])
            .env("HOME", temp.path())
            .env("XDG_CONFIG_HOME", config.parent().unwrap())
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"\n");
    }
}

#[test]
fn bad_shell_init_usage_is_an_error_not_shell_code() {
    for args in [
        vec!["shell-init"],
        vec!["shell-init", "fish"],
        vec!["shell-init", "bash", "extra"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_shoal"))
            .args(args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        assert!(output.stdout.is_empty());
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("usage: shoal shell-init bash|zsh")
        );
    }
}
