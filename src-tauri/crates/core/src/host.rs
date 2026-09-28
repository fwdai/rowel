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
}
