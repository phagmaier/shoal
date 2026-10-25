use std::sync::OnceLock;

use libghostty_vt::build_info;

/// The exact terminal core this binary links (M1 spec §5.3). Two cores decode
/// each other's snapshots only when these strings are byte-for-byte equal.
pub fn core_id() -> &'static str {
    static CORE: OnceLock<String> = OnceLock::new();
    CORE.get_or_init(|| {
        // Build settings are read from the linked library itself; a query the
        // library rejects shows up in the string rather than being guessed.
        let show = |value: Result<bool, libghostty_vt::Error>| match value {
            Ok(value) => value.to_string(),
            Err(error) => format!("error({error})"),
        };
        let version = build_info::version_string().map_or_else(
            |error| format!("error({error})"),
            |bytes| String::from_utf8_lossy(bytes).into_owned(),
        );
        let optimize = build_info::optimize_mode()
            .map_or_else(|error| format!("error({error})"), |mode| format!("{mode:?}"));
        format!(
            "ghostty={};libghostty-rs={};version={version};optimize={optimize};cpu={};simd={};kitty_graphics={};tmux={}",
            env!("SHOAL_GHOSTTY_REV"),
            env!("SHOAL_BINDINGS_REV"),
            env!("SHOAL_GHOSTTY_CPU"),
            show(build_info::supports_simd()),
            show(build_info::supports_kitty_graphics()),
            show(build_info::supports_tmux_control_mode()),
        )
    })
}
