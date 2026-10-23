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
/// - Saved with an explicit port (`https://bank.com:8443`): the page has to be
///   on that port. Another port on the same host is another server.
/// - Saved with no scheme at all (`bank.com`): the host is all the user said,
///   so the host is all that is checked.
///
/// A saved URL whose scheme is there but that does not parse is served to no
/// page.
pub fn site_matches(page_url: &str, saved: &str) -> bool {
    let Ok(page) = url::Url::parse(page_url) else {
        return false;
    };
    let Some(site) = page.host_str().map(str::to_ascii_lowercase) else {
        return false;
    };
    let saved = saved.trim();
    let Some((_, rest)) = saved.split_once("://") else {
        // No scheme: the `url_host` column is the whole of it.
        return host_matches(&site, saved.split('/').next().unwrap_or(""));
    };
    if !host_matches(&site, rest.split('/').next().unwrap_or("")) {
        return false;
    }
    let Ok(saved) = url::Url::parse(saved) else {
        return false;
    };
    if saved.scheme() == "https" && page.scheme() == "http" && !is_loopback(&page) {
        return false;
    }
    match saved.port() {
        Some(port) => page.port_or_known_default() == Some(port),
        None => true,
    }
}

/// Whether `url` names this machine: `localhost` or a loopback address.
fn is_loopback(url: &url::Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(domain)) => domain.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
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
    // host is another server. A default port written out is no constraint.
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

    // Typed without a scheme, the host is all the user said.
    #[test]
    fn a_login_saved_without_a_scheme_is_host_only() {
        assert!(site_matches("http://bank.com/", "bank.com"));
        assert!(site_matches("https://bank.com:8443/", "bank.com"));
        assert!(site_matches("http://bank.com:9000/", "bank.com:8443"));
        assert!(site_matches("https://www.bank.com/", "bank.com/login"));
        assert!(!site_matches("https://notbank.com/", "bank.com"));
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
