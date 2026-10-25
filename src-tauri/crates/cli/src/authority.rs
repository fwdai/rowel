//! The host of an HTTP target, by one rule. A grant names a host, a resolve
//! names a host, the proxy addresses a host; if any of them wrote it
//! differently — one with the port, one without, one in capitals — a grant
//! for `localhost:3000` would never match the request it was asked for.
//! Everything that writes the host of an HTTP target goes through here.

/// `authority` as the host of a target for `scheme`: lower case, the port
/// kept when it is not the scheme's default (443 for `https`, 80 for
/// `http`) and dropped when it is, an IPv6 literal's brackets kept.
pub fn authority(scheme: &str, authority: &str) -> String {
    let authority = authority.to_ascii_lowercase();
    let default_port = match scheme {
        "https" => "443",
        "http" => "80",
        _ => "",
    };
    let (host, port) = split_port(&authority);
    match port {
        Some(port) if port == default_port => host.to_string(),
        _ => authority,
    }
}

/// `host` and its port when it has one: after the last colon, unless the
/// host is a bare IPv6 literal (more than one colon, no brackets) or the
/// bracketed literal has no port after the bracket.
fn split_port(authority: &str) -> (&str, Option<&str>) {
    if authority.starts_with('[') {
        return match authority.split_once(']') {
            Some((host, rest)) => (
                &authority[..=host.len()],
                rest.strip_prefix(':').filter(|p| !p.is_empty()),
            ),
            None => (authority, None),
        };
    }
    match authority.rsplit_once(':') {
        Some((host, port))
            if !host.contains(':')
                && !port.is_empty()
                && port.bytes().all(|b| b.is_ascii_digit()) =>
        {
            (host, Some(port))
        }
        _ => (authority, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_port_is_dropped_and_any_other_kept() {
        assert_eq!(authority("https", "api.example.com:443"), "api.example.com");
        assert_eq!(authority("https", "API.Example.com"), "api.example.com");
        assert_eq!(authority("https", "localhost:3000"), "localhost:3000");
        assert_eq!(authority("http", "example.com:80"), "example.com");
        assert_eq!(authority("https", "example.com:80"), "example.com:80");
        assert_eq!(authority("http", "example.com:443"), "example.com:443");
        assert_eq!(authority("https", "127.0.0.1:8443"), "127.0.0.1:8443");
    }

    #[test]
    fn an_ipv6_literal_keeps_its_brackets() {
        assert_eq!(authority("https", "[::1]:8443"), "[::1]:8443");
        assert_eq!(authority("https", "[::1]:443"), "[::1]");
        assert_eq!(authority("https", "[::1]"), "[::1]");
        assert_eq!(authority("https", "[2001:DB8::1]:443"), "[2001:db8::1]");
    }

    #[test]
    fn what_is_not_a_port_stays_part_of_the_host() {
        assert_eq!(authority("https", "host:"), "host:");
        assert_eq!(authority("https", "host:abc"), "host:abc");
        assert_eq!(authority("https", "::1"), "::1");
    }
}
