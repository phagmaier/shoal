/// Parts kept when a directory is shortened to `…/<last parts>`.
const KEEP: usize = 3;

/// `pwd` with `home` written as `~`, cut to its last three parts after `…/`.
pub fn display(pwd: &str, home: Option<&str>) -> String {
    // A home of `/` would turn every path into `~`.
    let home = home
        .map(|home| home.trim_end_matches('/'))
        .filter(|home| !home.is_empty());
    let path = match home.and_then(|home| pwd.strip_prefix(home)) {
        Some("") => "~".to_owned(),
        Some(rest) if rest.starts_with('/') => format!("~{rest}"),
        _ => pwd.to_owned(),
    };
    let parts: Vec<_> = path.split('/').filter(|part| !part.is_empty()).collect();
    if parts.len() > KEEP {
        format!("…/{}", parts[parts.len() - KEEP..].join("/"))
    } else {
        path
    }
}

#[cfg(test)]
mod tests {
    use super::display;

    #[test]
    fn writes_home_as_tilde_and_keeps_three_parts() {
        let home = Some("/home/parker");
        for (pwd, shown) in [
            ("/home/parker", "~"),
            ("/home/parker/code", "~/code"),
            ("/home/parker/code/shoal", "~/code/shoal"),
            ("/home/parker/code/shoal/crates", "…/code/shoal/crates"),
            ("/home/parker/code/shoal/crates/cli", "…/shoal/crates/cli"),
            ("/home/parkerx/code", "/home/parkerx/code"),
            ("/", "/"),
            ("/usr/local/share", "/usr/local/share"),
            ("/usr/local/share/doc", "…/local/share/doc"),
        ] {
            assert_eq!(display(pwd, home), shown, "{pwd}");
        }
    }

    #[test]
    fn tolerates_unusual_homes() {
        assert_eq!(
            display("/home/parker/code", Some("/home/parker/")),
            "~/code"
        );
        assert_eq!(display("/etc", Some("/")), "/etc");
        assert_eq!(display("/etc", Some("")), "/etc");
        assert_eq!(display("/home/parker", None), "/home/parker");
    }
}
