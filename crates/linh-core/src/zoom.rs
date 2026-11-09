//! Rewrites a Zoom web-join URL into the Zoom desktop app's `zoommtg://`
//! deep link, per the plan's Contract section.

use url::Url;

/// Rewrites `https://<host>.zoom.us/j/<id>?pwd=x` (where `<host>` may be
/// absent, i.e. bare `zoom.us`) into
/// `zoommtg://<host>.zoom.us/join?action=join&confno=<id>&pwd=x`,
/// omitting the `&pwd=...` segment when the original URL has no `pwd`
/// parameter. Returns `None` when `url` is not a convertible Zoom join
/// link: wrong scheme, a host that is not `zoom.us` or `*.zoom.us`, a
/// path that is not exactly `/j/<id>`, or a meeting id that is not all
/// ASCII digits.
pub fn zoom_app_url(url: &str) -> Option<String> {
    let parsed = Url::parse(url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }

    let host = parsed.host_str()?;
    if host != "zoom.us" && !host.ends_with(".zoom.us") {
        return None;
    }

    let mut segments = parsed.path_segments()?;
    if segments.next()? != "j" {
        return None;
    }
    let id = segments.next()?;
    if segments.next().is_some() {
        return None;
    }
    if id.is_empty() || !id.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }

    // Read the `pwd` param from the raw query string (not
    // `query_pairs()`, which percent-decodes) so the value is carried
    // over byte-for-byte rather than risking a re-encoding mismatch.
    let pwd = parsed
        .query()
        .and_then(|q| q.split('&').find_map(|pair| pair.strip_prefix("pwd=")));

    let mut out = format!("zoommtg://{host}/join?action=join&confno={id}");
    if let Some(pwd) = pwd {
        out.push_str("&pwd=");
        out.push_str(pwd);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_subdomain_host_with_pwd() {
        assert_eq!(
            zoom_app_url("https://us02web.zoom.us/j/123456789?pwd=abc"),
            Some("zoommtg://us02web.zoom.us/join?action=join&confno=123456789&pwd=abc".to_string())
        );
    }

    #[test]
    fn rewrites_bare_domain_without_pwd() {
        assert_eq!(
            zoom_app_url("https://zoom.us/j/123456789"),
            Some("zoommtg://zoom.us/join?action=join&confno=123456789".to_string())
        );
    }

    #[test]
    fn preserves_pwd_value_verbatim() {
        assert_eq!(
            zoom_app_url("https://zoom.us/j/42?pwd=a%2Fb"),
            Some("zoommtg://zoom.us/join?action=join&confno=42&pwd=a%2Fb".to_string())
        );
    }

    #[test]
    fn ignores_other_query_params_and_keeps_pwd() {
        assert_eq!(
            zoom_app_url("https://zoom.us/j/42?uname=Jane&pwd=xyz"),
            Some("zoommtg://zoom.us/join?action=join&confno=42&pwd=xyz".to_string())
        );
    }

    #[test]
    fn rejects_non_zoom_host() {
        assert_eq!(zoom_app_url("https://notzoom.us/j/123"), None);
    }

    #[test]
    fn rejects_host_merely_containing_zoom_us_as_a_suffix_of_a_label() {
        assert_eq!(zoom_app_url("https://evilzoom.us/j/123"), None);
    }

    #[test]
    fn rejects_non_digit_meeting_id() {
        assert_eq!(zoom_app_url("https://zoom.us/j/abc123"), None);
    }

    #[test]
    fn rejects_empty_meeting_id() {
        assert_eq!(zoom_app_url("https://zoom.us/j/"), None);
    }

    #[test]
    fn rejects_non_join_path() {
        assert_eq!(zoom_app_url("https://zoom.us/pricing"), None);
    }

    #[test]
    fn rejects_extra_path_segments_after_the_id() {
        assert_eq!(zoom_app_url("https://zoom.us/j/123/extra"), None);
    }

    #[test]
    fn rejects_unparseable_url() {
        assert_eq!(zoom_app_url("not a url"), None);
    }

    #[test]
    fn rejects_non_http_scheme() {
        assert_eq!(zoom_app_url("zoommtg://zoom.us/j/123"), None);
    }
}
