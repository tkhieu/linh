use url::Url;

/// Parses `arg` as a URL and returns its normalized form if (and only if)
/// its scheme is `http` or `https`. Everything else (unparseable strings,
/// `mailto:`, `file:`, flag-like arguments, ...) is `None`.
pub fn filter_routable_url(arg: &str) -> Option<String> {
    let url = Url::parse(arg).ok()?;
    match url.scheme() {
        "http" | "https" => Some(url.to_string()),
        _ => None,
    }
}

/// Returns the first routable (`http`/`https`) URL among `args`, if any.
pub fn first_routable_url<'a, I>(args: I) -> Option<String>
where
    I: IntoIterator<Item = &'a String>,
{
    args.into_iter().find_map(|arg| filter_routable_url(arg))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_http_and_https() {
        assert_eq!(
            filter_routable_url("http://example.com"),
            Some("http://example.com/".to_string())
        );
        assert!(filter_routable_url("https://example.com/path?q=1").is_some());
    }

    #[test]
    fn rejects_other_schemes() {
        assert_eq!(filter_routable_url("mailto:foo@bar.com"), None);
        assert_eq!(filter_routable_url("file:///etc/passwd"), None);
    }

    #[test]
    fn rejects_unparseable_strings() {
        assert_eq!(filter_routable_url("not a url"), None);
        assert_eq!(filter_routable_url("--flag"), None);
        assert_eq!(filter_routable_url(""), None);
    }

    #[test]
    fn first_routable_url_skips_non_matching_args() {
        let args = vec![
            "/usr/bin/linh".to_string(),
            "--some-flag".to_string(),
            "https://example.com".to_string(),
        ];
        assert_eq!(
            first_routable_url(&args),
            Some("https://example.com/".to_string())
        );
    }

    #[test]
    fn first_routable_url_none_when_no_url_present() {
        let args = vec!["/usr/bin/linh".to_string(), "--flag".to_string()];
        assert_eq!(first_routable_url(&args), None);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_protocol_command_url_reaches_single_instance_dispatch() {
        let args = vec![
            r"C:\Users\Alice\AppData\Local\Programs\Linh\linh.exe".to_string(),
            "https://example.com/path?q=1&n=2".to_string(),
        ];
        assert_eq!(
            first_routable_url(&args),
            Some("https://example.com/path?q=1&n=2".to_string())
        );
    }
}
