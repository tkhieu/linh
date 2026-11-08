//! URL pattern matching: plain domains (matching the domain and its
//! subdomains), wildcard domain/path patterns, case-insensitive host
//! comparison.

/// Returns true if `pattern` matches the given host and path.
///
/// `pattern` is either:
/// - a plain domain, e.g. `company.com`, which matches `company.com` and
///   any subdomain such as `a.company.com`;
/// - a wildcard domain, e.g. `*.company.com`, which matches only
///   subdomains (not the bare domain), using `*` as a glob wildcard;
/// - either of the above followed by `/` and a path glob, e.g.
///   `example.com/path*`.
///
/// Host comparison is case-insensitive.
pub fn pattern_matches(pattern: &str, host: &str, path: &str) -> bool {
    let pattern = pattern.to_lowercase();
    let host = host.to_lowercase();

    let (domain_pattern, path_pattern) = match pattern.split_once('/') {
        Some((domain, rest)) => (domain, Some(rest)),
        None => (pattern.as_str(), None),
    };

    if !domain_matches(domain_pattern, &host) {
        return false;
    }

    match path_pattern {
        Some(pp) => {
            let actual_path = path.trim_start_matches('/');
            let pp = pp.trim_start_matches('/');
            glob_match(pp, actual_path)
        }
        None => true,
    }
}

fn domain_matches(domain_pattern: &str, host: &str) -> bool {
    if domain_pattern.contains('*') {
        glob_match(domain_pattern, host)
    } else {
        host == domain_pattern || host.ends_with(&format!(".{domain_pattern}"))
    }
}

/// Minimal glob matcher supporting `*` (matches any sequence of
/// characters, including none). Anchored at both ends.
fn glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (plen, tlen) = (p.len(), t.len());

    let mut dp = vec![vec![false; tlen + 1]; plen + 1];
    dp[0][0] = true;
    for i in 1..=plen {
        if p[i - 1] == '*' {
            dp[i][0] = dp[i - 1][0];
        }
    }
    for i in 1..=plen {
        for j in 1..=tlen {
            dp[i][j] = if p[i - 1] == '*' {
                dp[i - 1][j] || dp[i][j - 1]
            } else {
                dp[i - 1][j - 1] && p[i - 1] == t[j - 1]
            };
        }
    }
    dp[plen][tlen]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_domain_matches_itself_and_subdomains() {
        assert!(pattern_matches("company.com", "company.com", "/"));
        assert!(pattern_matches("company.com", "a.company.com", "/"));
        assert!(pattern_matches("company.com", "a.b.company.com", "/"));
        assert!(!pattern_matches("company.com", "notcompany.com", "/"));
        assert!(!pattern_matches("company.com", "company.com.evil.com", "/"));
    }

    #[test]
    fn wildcard_domain_matches_only_subdomains() {
        assert!(pattern_matches("*.company.com", "a.company.com", "/"));
        assert!(pattern_matches("*.company.com", "a.b.company.com", "/"));
        assert!(!pattern_matches("*.company.com", "company.com", "/"));
    }

    #[test]
    fn host_comparison_is_case_insensitive() {
        assert!(pattern_matches("Company.COM", "company.com", "/"));
        assert!(pattern_matches("company.com", "COMPANY.COM", "/"));
    }

    #[test]
    fn path_wildcard_matches_prefix() {
        assert!(pattern_matches(
            "example.com/path*",
            "example.com",
            "/path/foo"
        ));
        assert!(pattern_matches("example.com/path*", "example.com", "/path"));
        assert!(!pattern_matches(
            "example.com/path*",
            "example.com",
            "/other"
        ));
    }

    #[test]
    fn path_pattern_requires_domain_match_too() {
        assert!(!pattern_matches(
            "example.com/path*",
            "notexample.com",
            "/path"
        ));
    }

    #[test]
    fn meet_google_com_matches() {
        assert!(pattern_matches("meet.google.com", "meet.google.com", "/"));
        assert!(!pattern_matches("meet.google.com", "google.com", "/"));
    }
}
