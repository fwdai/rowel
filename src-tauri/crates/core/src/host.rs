//! Which logins belong to a site: the one rule the browser extension host and
//! the iOS AutoFill extension both fill by.

/// The host a page URL names, lowercase, or `None` for anything that is not a
/// URL with one.
pub fn site_host(url: &str) -> Option<String> {
    url::Url::parse(url)
        .ok()?
        .host_str()
        .map(str::to_ascii_lowercase)
}

/// Whether a login stored for `entry` (its `url_host` column) belongs to the
/// page at `site`: the same host, or a subdomain of it, with a leading `www.`
/// on either side not counting. `accounts.example.com` gets the login saved
/// for `example.com`; `example.com` does not get one saved for
/// `accounts.example.com`, and `notexample.com` gets neither. A login stored
/// under a public suffix — `com`, `co.uk`, whether typed or imported — is
/// served to no site beneath it: that is not a parent, it is everyone.
///
/// The column is the website field cut after its scheme and before its first
/// `/` — so a port, a query or a user name typed there stays in it — where
/// `site` is a browser's host and never carries any of those. Read back as a
/// URL, the column comes out as a bare host the way `site` does, by the one
/// parser, rather than by stripping each of those by hand.
pub fn host_matches(site: &str, entry: &str) -> bool {
    let Some(entry) = entry_host(entry) else {
        return false;
    };
    let site = site.trim_start_matches("www.");
    let entry = entry.trim_start_matches("www.");
    site == entry || (site.ends_with(&format!(".{entry}")) && !is_public_suffix(entry))
}

/// Whether a login saved for `saved` (its website field, as the user typed or
/// an importer wrote it) may be served to the page at `page_url`: its host
/// matches by [`host_matches`], and the page is no weaker than what was saved.
///
/// - Saved for `https://` and asked for by an `http://` page: refused, since
///   anyone on the network can serve that page. A page on the machine itself
///   (`localhost`, `127.0.0.1`, `::1`) is the one exception.
/// - Saved with an explicit port (`https://bank.com:8443`, `bank.com:8443`,
///   `https://bank.com:443`): the page has to be on that port, with or
///   without a scheme typed, and even when it is the scheme's default.
///   Another port on the same host is another server.
/// - Saved with no port: any port on the host.
/// - Saved with no scheme (`bank.com`): no scheme is checked.
///
/// A saved value whose scheme or port does not read as one is served to no
/// page.
pub fn site_matches(page_url: &str, saved: &str) -> bool {
    let Ok(page) = url::Url::parse(page_url) else {
        return false;
    };
    let Some(site) = page.host_str().map(str::to_ascii_lowercase) else {
        return false;
    };
    let Some(saved) = SavedSite::parse(saved) else {
        return false;
    };
    if !host_matches(&site, saved.host) {
        return false;
    }
    if saved.scheme.as_deref() == Some("https") && page.scheme() == "http" && !is_loopback(&page) {
        return false;
    }
    match saved.port {
        Some(port) => page.port_or_known_default() == Some(port),
        None => true,
    }
}

/// What a website field said, read as the user wrote it. Not through
/// `url::Url`: that parser drops a port equal to its scheme's default, so a
/// written-out `:443` would read as no port at all.
#[derive(Debug, PartialEq)]
struct SavedSite<'a> {
    /// Lowercase, when one was typed before `://`.
    scheme: Option<String>,
    /// The host as written — brackets kept on an IPv6 address — for
    /// [`host_matches`] to normalise.
    host: &'a str,
    /// When one was written out, whatever its value.
    port: Option<u16>,
}

impl<'a> SavedSite<'a> {
    /// The scheme (if `://` comes before the authority ends), then the
    /// authority up to the first `/`, `\`, `?` or `#`, less any `user@`.
    /// `None` when the scheme or the port is not one: such a value matches
    /// nothing.
    fn parse(saved: &'a str) -> Option<Self> {
        const AUTHORITY_ENDS: [char; 4] = ['/', '\\', '?', '#'];
        let saved = saved.trim();
        let (scheme, rest) = match saved.find("://") {
            Some(at) if !saved[..at].contains(AUTHORITY_ENDS) => {
                let scheme = &saved[..at];
                if !is_scheme(scheme) {
                    return None;
                }
                (Some(scheme.to_ascii_lowercase()), &saved[at + 3..])
            }
            _ => (None, saved),
        };
        let authority = rest.split(AUTHORITY_ENDS).next().unwrap_or_default();
        let authority = authority.rsplit_once('@').map_or(authority, |(_, a)| a);
        let (host, port) = match authority.strip_prefix('[') {
            Some(bracketed) => {
                let close = bracketed.find(']')? + 2;
                let (host, tail) = authority.split_at(close);
                match tail {
                    "" => (host, None),
                    tail => (host, Some(tail.strip_prefix(':')?)),
                }
            }
            None => match authority.split_once(':') {
                Some((host, port)) => (host, Some(port)),
                None => (authority, None),
            },
        };
        let port = match port {
            // `host:` with nothing after it names no port, as a URL reads it.
            None | Some("") => None,
            Some(port) if port.bytes().all(|b| b.is_ascii_digit()) => Some(port.parse().ok()?),
            Some(_) => return None,
        };
        Some(Self { scheme, host, port })
    }
}

/// A URL scheme: a letter, then letters, digits, `+`, `-` or `.`.
fn is_scheme(scheme: &str) -> bool {
    let mut chars = scheme.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
}

/// Whether `url` names this machine: `localhost` or a loopback address.
fn is_loopback(url: &url::Url) -> bool {
    url.host_str().is_some_and(is_loopback_host)
}

/// Whether `host`, as a URL spells it, is this machine: `localhost`, an
/// address in `127.0.0.0/8`, or `[::1]`. The one rule for where plaintext is
/// allowed — a page that may fill an https login over http, a Rowel Server
/// reached over http, a broker over `ws://` — since traffic that never
/// leaves the machine has no one in the middle to read or alter it.
pub fn is_loopback_host(host: &str) -> bool {
    let bare = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);
    bare.eq_ignore_ascii_case("localhost")
        || bare
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// The host in a `url_host` column, or `None` when what was typed there does
/// not read as one.
fn entry_host(entry: &str) -> Option<String> {
    let entry = entry.trim();
    if entry.is_empty() || entry.contains("://") {
        return None;
    }
    site_host(&format!("https://{entry}"))
}

/// Whether `host` is a public suffix — `com`, `co.uk` — under which anyone's
/// site could live. What keeps a parent-domain match honest, here and for the
/// rpId a passkey may claim.
pub fn is_public_suffix(host: &str) -> bool {
    public_suffix::DEFAULT_PROVIDER.is_effective_tld(host)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_this_machine_is_a_loopback_host() {
        for host in [
            "localhost",
            "LocalHost",
            "127.0.0.1",
            "127.8.9.10",
            "[::1]",
            "::1",
        ] {
            assert!(is_loopback_host(host), "{host}");
        }
        for host in [
            "example.com",
            "localhost.example.com",
            "10.0.0.1",
            "0.0.0.0",
            "[::]",
            "[2001:db8::1]",
            "",
        ] {
            assert!(!is_loopback_host(host), "{host}");
        }
    }

    #[test]
    fn site_host_is_the_lowercase_host_of_a_url() {
        assert_eq!(
            site_host("https://Accounts.Google.com/signin?x=1").as_deref(),
            Some("accounts.google.com")
        );
        assert_eq!(
            site_host("http://192.168.1.1/").as_deref(),
            Some("192.168.1.1")
        );
        assert_eq!(site_host("github.com"), None);
        assert_eq!(site_host(""), None);
    }

    #[test]
    fn a_login_matches_its_host_and_subdomains_but_not_its_lookalikes() {
        assert!(host_matches("github.com", "github.com"));
        assert!(host_matches("github.com", "GitHub.com"));
        assert!(host_matches("www.github.com", "github.com"));
        assert!(host_matches("github.com", "www.github.com"));
        assert!(host_matches("accounts.google.com", "google.com"));
        assert!(!host_matches("google.com", "accounts.google.com"));
        assert!(!host_matches("notgithub.com", "github.com"));
        assert!(!host_matches("github.com", ""));
        assert!(!host_matches("github.com", "  "));
        // A public suffix is not a parent: a login stored under one — a malformed
        // website, an import — is served to no site beneath it.
        assert!(!host_matches("github.com", "com"));
        assert!(!host_matches("bank.co.uk", "co.uk"));
        assert!(
            host_matches("co.uk", "co.uk"),
            "the suffix itself, as a site, still is"
        );
        assert!(host_matches("online.bank.co.uk", "bank.co.uk"));
    }

    #[test]
    fn a_login_matches_whatever_else_its_website_field_carried() {
        // The column is the website cut before its first `/`, so all of these
        // are what a typed URL leaves in it.
        assert!(host_matches("example.com", "example.com:8443"));
        assert!(host_matches("example.com", "user@example.com"));
        assert!(host_matches("example.com", "user:pw@example.com:8443"));
        assert!(host_matches("example.com", "example.com?next=1"));
        assert!(host_matches("example.com", "example.com#top"));
        assert!(!host_matches("example.com", "example.com:notaport"));
        assert!(host_matches("192.168.1.1", "192.168.1.1:8080"));
    }

    // The host rule is unchanged underneath: subdomains still get the
    // parent's login, lookalikes and public suffixes still get nothing.
    #[test]
    fn a_site_match_is_a_host_match_first() {
        assert!(site_matches("https://bank.com/login", "https://bank.com"));
        assert!(site_matches(
            "https://online.bank.com/",
            "https://bank.com/"
        ));
        assert!(!site_matches(
            "https://bank.com/",
            "https://online.bank.com"
        ));
        assert!(!site_matches("https://notbank.com/", "https://bank.com"));
        assert!(!site_matches("https://bank.co.uk/", "https://co.uk"));
        assert!(!site_matches("not a url", "https://bank.com"));
        assert!(!site_matches("https://bank.com/", ""));
        // A scheme that does not parse as a URL is served nowhere.
        assert!(!site_matches(
            "https://bank.com/",
            "https://bank.com:notaport"
        ));
    }

    // Saved for HTTPS, never served to plain HTTP — the page anyone on the
    // network can stand up — except on the machine itself.
    #[test]
    fn an_https_login_is_not_served_to_an_http_page() {
        assert!(!site_matches("http://bank.com/login", "https://bank.com"));
        assert!(!site_matches("http://online.bank.com/", "https://bank.com"));
        assert!(!site_matches("HTTP://BANK.COM/", "HTTPS://bank.com"));
        assert!(site_matches("https://bank.com/", "http://bank.com"));
        assert!(site_matches("http://bank.com/", "http://bank.com"));

        assert!(site_matches(
            "http://localhost:3000/",
            "https://localhost:3000"
        ));
        assert!(site_matches("http://127.0.0.1/", "https://127.0.0.1"));
        assert!(site_matches("http://[::1]/", "https://[::1]"));
    }

    // An explicit port is part of what was saved: another port on the same
    // host is another server. A default port written out is that port.
    #[test]
    fn an_explicit_port_has_to_match() {
        assert!(site_matches(
            "https://bank.com:8443/",
            "https://bank.com:8443/login"
        ));
        assert!(!site_matches("https://bank.com/", "https://bank.com:8443"));
        assert!(!site_matches(
            "https://bank.com:9443/",
            "https://bank.com:8443"
        ));
        assert!(site_matches(
            "https://bank.com:443/",
            "https://bank.com:443"
        ));
        assert!(site_matches("https://bank.com/", "https://bank.com:443"));
        assert!(site_matches(
            "http://bank.com:8080/",
            "http://bank.com:8080"
        ));
        // No port saved: any port on the host, as before.
        assert!(site_matches("https://bank.com:8443/", "https://bank.com"));
    }

    // A port written out is enforced even when it is the scheme's default,
    // which the URL parser would otherwise drop.
    #[test]
    fn an_explicit_default_port_is_still_enforced() {
        let served: Vec<_> = [
            ("https://bank.com:8443/", "https://bank.com:443"),
            ("http://localhost:3000/", "https://localhost:443"),
            ("http://127.0.0.1:3000/", "https://127.0.0.1:443"),
            ("http://bank.com:8080/", "http://bank.com:80"),
        ]
        .into_iter()
        .filter(|(page, saved)| site_matches(page, saved))
        .collect();
        assert!(served.is_empty(), "served to another port: {served:?}");

        // On the default port itself, written out or not, they still match.
        assert!(site_matches("http://bank.com/", "http://bank.com:80"));
        assert!(site_matches("https://localhost/", "https://localhost:443"));
        assert!(site_matches("https://[::1]/", "https://[::1]:443"));
        assert!(!site_matches("https://[::1]:8443/", "https://[::1]:443"));
        assert!(!site_matches("http://[::1]:3000/", "https://[::1]:443"));
    }

    // Typed without a scheme, no scheme is checked; a port typed is still
    // a port, as explicit as one after `https://`.
    #[test]
    fn a_login_saved_without_a_scheme_checks_no_scheme() {
        assert!(site_matches("http://bank.com/", "bank.com"));
        assert!(site_matches("https://bank.com:8443/", "bank.com"));
        assert!(site_matches("http://bank.com:8443/", "bank.com:8443"));
        assert!(!site_matches("http://bank.com:9000/", "bank.com:8443"));
        assert!(!site_matches("https://bank.com:8443/", "bank.com:443"));
        assert!(site_matches("https://bank.com/", "bank.com:443"));
        assert!(!site_matches("http://localhost:3000/", "localhost:443"));
        assert!(site_matches("http://localhost:3000/", "localhost:3000"));
        assert!(site_matches("https://www.bank.com/", "bank.com/login"));
        assert!(!site_matches("https://notbank.com/", "bank.com"));
    }

    #[test]
    fn a_saved_site_is_read_as_written() {
        let site = |scheme: Option<&str>, host, port| SavedSite {
            scheme: scheme.map(str::to_string),
            host,
            port,
        };
        let parse = SavedSite::parse;
        assert_eq!(
            parse(" HTTPS://bank.com:443/login "),
            Some(site(Some("https"), "bank.com", Some(443)))
        );
        assert_eq!(
            parse("https://bank.com"),
            Some(site(Some("https"), "bank.com", None))
        );
        assert_eq!(
            parse("bank.com:8443?next=/x"),
            Some(site(None, "bank.com", Some(8443)))
        );
        assert_eq!(
            parse("https://user:p@ss@bank.com:80#top"),
            Some(site(Some("https"), "bank.com", Some(80)))
        );
        assert_eq!(
            parse("https://[::1]:443/"),
            Some(site(Some("https"), "[::1]", Some(443)))
        );
        assert_eq!(
            parse("[2001:db8::1]"),
            Some(site(None, "[2001:db8::1]", None))
        );
        // An empty port is no port, as the URL parser reads it.
        assert_eq!(
            parse("https://bank.com:/"),
            Some(site(Some("https"), "bank.com", None))
        );
        // A `\` ends the authority, as it does for the URL parser.
        assert_eq!(
            parse("https://bank.com\\@evil.com"),
            Some(site(Some("https"), "bank.com", None))
        );
        // A `://` after the authority ended is not a scheme.
        assert_eq!(
            parse("bank.com/?to=https://evil.com"),
            Some(site(None, "bank.com", None))
        );

        // Not a port, or not a scheme: nothing.
        assert_eq!(parse("https://bank.com:notaport"), None);
        assert_eq!(parse("https://bank.com:65536"), None);
        assert_eq!(parse("https://bank.com:+443"), None);
        assert_eq!(parse("https://[::1]x"), None);
        assert_eq!(parse("https://[::1"), None);
        assert_eq!(parse("ht tp://bank.com"), None);
    }

    // Internationalised names compare in their ASCII form, whichever way each
    // side spelled them.
    #[test]
    fn idn_hosts_match_in_either_spelling() {
        assert!(site_matches(
            "https://xn--bcher-kva.example/",
            "https://bücher.example"
        ));
        assert!(site_matches(
            "https://bücher.example/",
            "https://xn--bcher-kva.example"
        ));
        assert!(site_matches(
            "https://shop.bücher.example/",
            "bücher.example"
        ));
        assert!(!site_matches(
            "http://bücher.example/",
            "https://bücher.example"
        ));
        assert!(!site_matches(
            "https://bucher.example/",
            "https://bücher.example"
        ));
    }

    #[test]
    fn ipv6_hosts_match_with_their_ports() {
        assert!(site_matches(
            "https://[2001:db8::1]/",
            "https://[2001:db8::1]"
        ));
        assert!(site_matches(
            "https://[2001:db8::1]:8443/",
            "https://[2001:db8::1]:8443/x"
        ));
        assert!(!site_matches(
            "https://[2001:db8::1]/",
            "https://[2001:db8::1]:8443"
        ));
        assert!(!site_matches(
            "http://[2001:db8::1]/",
            "https://[2001:db8::1]"
        ));
        assert!(!site_matches(
            "https://[2001:db8::2]/",
            "https://[2001:db8::1]"
        ));
    }
}
