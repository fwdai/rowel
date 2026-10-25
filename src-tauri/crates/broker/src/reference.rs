//! The placeholder: how an agent names a secret it never holds.
//! `rowel://<lockbox>/<item>/<field>` is what goes into a header, an
//! environment file or a command line in place of the value, and what a
//! sidecar hands back to the host to be resolved under a grant (`spec/04`).
//!
//! A placeholder is not a capability. `lockbox` and `item` are an id or a
//! slug, opaque here — the host resolves them against the lockbox it has
//! open — and knowing the name of a secret buys nothing without a grant that
//! covers it. The parse is strict, because a name the host had to guess at
//! could resolve to something the grant did not mean.

use std::fmt;

pub const SCHEME: &str = "rowel://";

/// Which secret of an item a placeholder names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Field {
    Password,
    Otp,
    ApiKey,
    PrivateKey,
    /// One variable of an `env` item's file, by its name.
    Body(String),
    Number,
    Cvc,
}

impl Field {
    /// The field named `text`, in the words the protocol gives them. A `body`
    /// without a key names nothing: an env item's whole file is never one
    /// value.
    pub fn parse(text: &str) -> Option<Field> {
        match text {
            "password" => Some(Field::Password),
            "otp" => Some(Field::Otp),
            "apiKey" => Some(Field::ApiKey),
            "privateKey" => Some(Field::PrivateKey),
            "number" => Some(Field::Number),
            "cvc" => Some(Field::Cvc),
            _ => {
                let key = text.strip_prefix("body.")?;
                (!key.is_empty() && !key.contains('/')).then(|| Field::Body(key.to_string()))
            }
        }
    }

    /// The entry kinds (the store's `kind` column: `login`, `apikey`, `ssh`,
    /// `card`, `env`) that carry this field.
    pub fn kinds(&self) -> &'static [&'static str] {
        match self {
            Field::Password | Field::Otp => &["login"],
            Field::ApiKey => &["apikey"],
            Field::PrivateKey => &["ssh"],
            Field::Body(_) => &["env"],
            Field::Number | Field::Cvc => &["card"],
        }
    }
}

impl fmt::Display for Field {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Field::Password => f.write_str("password"),
            Field::Otp => f.write_str("otp"),
            Field::ApiKey => f.write_str("apiKey"),
            Field::PrivateKey => f.write_str("privateKey"),
            Field::Body(key) => write!(f, "body.{key}"),
            Field::Number => f.write_str("number"),
            Field::Cvc => f.write_str("cvc"),
        }
    }
}

/// A parsed placeholder: which lockbox, which item in it, which of its
/// secrets.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reference {
    pub lockbox: String,
    pub item: String,
    pub field: Field,
}

impl Reference {
    /// `text` as a placeholder: the scheme, then exactly three non-empty
    /// segments, the last a [`Field`]. A trailing slash, a query, a fragment,
    /// a segment too few or too many, or a field the protocol does not name
    /// is not a placeholder at all, rather than nearly one.
    pub fn parse(text: &str) -> Option<Reference> {
        let rest = text.strip_prefix(SCHEME)?;
        if rest.contains(['?', '#']) {
            return None;
        }
        let mut segments = rest.split('/');
        let (lockbox, item, field) = (segments.next()?, segments.next()?, segments.next()?);
        if segments.next().is_some() || lockbox.is_empty() || item.is_empty() {
            return None;
        }
        Some(Reference {
            lockbox: lockbox.to_string(),
            item: item.to_string(),
            field: Field::parse(field)?,
        })
    }
}

impl fmt::Display for Reference {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{SCHEME}{}/{}/{}", self.lockbox, self.item, self.field)
    }
}

/// The fields of an item of `kind` a grant could cover, by the names the
/// agent protocol gives them. The kinds are the store's `kind` column, which
/// is the frontend's `EntryType`: `ssh`, not `sshkey`. A kind with nothing
/// an agent could be served — a note, an identity — has none.
pub fn fields_for(kind: &str) -> &'static [&'static str] {
    match kind {
        "login" => &["password", "otp"],
        "apikey" => &["apiKey"],
        "ssh" => &["privateKey"],
        "card" => &["number", "cvc"],
        "env" => &["body"],
        _ => &[],
    }
}

/// A title as a slug, the way an agent writes an item's name in a
/// placeholder: lowercased, every run of characters that is not alphanumeric
/// collapsed to one `-`, none leading or trailing. `Stripe (live)` is
/// `stripe-live`.
pub fn slug(title: &str) -> String {
    let mut out = String::with_capacity(title.len());
    let mut gap = false;
    for c in title.chars() {
        if c.is_alphanumeric() {
            if gap && !out.is_empty() {
                out.push('-');
            }
            gap = false;
            out.extend(c.to_lowercase());
        } else {
            gap = true;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_placeholder_parses_and_prints_the_same() {
        for (text, lockbox, item, field) in [
            (
                "rowel://work/stripe/apiKey",
                "work",
                "stripe",
                Field::ApiKey,
            ),
            ("rowel://lb/gh/password", "lb", "gh", Field::Password),
            ("rowel://lb/gh/otp", "lb", "gh", Field::Otp),
            (
                "rowel://lb/deploy/privateKey",
                "lb",
                "deploy",
                Field::PrivateKey,
            ),
            (
                "rowel://lb/ci/body.AWS_SECRET",
                "lb",
                "ci",
                Field::Body("AWS_SECRET".into()),
            ),
            ("rowel://lb/visa/number", "lb", "visa", Field::Number),
            ("rowel://lb/visa/cvc", "lb", "visa", Field::Cvc),
            (
                "rowel://0123abcd/fedc9876/password",
                "0123abcd",
                "fedc9876",
                Field::Password,
            ),
        ] {
            let parsed = Reference::parse(text).unwrap_or_else(|| panic!("{text}"));
            assert_eq!(
                parsed,
                Reference {
                    lockbox: lockbox.into(),
                    item: item.into(),
                    field,
                }
            );
            assert_eq!(parsed.to_string(), text);
        }
    }

    #[test]
    fn what_is_not_a_placeholder_is_none() {
        for text in [
            "",
            "rowel://",
            "ROWEL://lb/gh/password",
            "rowel:/lb/gh/password",
            "https://lb/gh/password",
            "lb/gh/password",
            "rowel://lb/gh",
            "rowel://lb/gh/password/",
            "rowel://lb/gh/password/extra",
            "rowel:////password",
            "rowel:///gh/password",
            "rowel://lb//password",
            "rowel://lb/gh/",
            "rowel://lb/gh/password?x=1",
            "rowel://lb/gh/password#top",
            "rowel://lb?x/gh/password",
            "rowel://lb/gh/secret",
            "rowel://lb/gh/Password",
            "rowel://lb/gh/body",
            "rowel://lb/gh/body.",
            "rowel://lb/gh/body.A/B",
        ] {
            assert_eq!(Reference::parse(text), None, "{text:?}");
        }
    }

    #[test]
    fn a_field_is_one_of_the_protocols_words() {
        assert_eq!(Field::parse("body"), None);
        assert_eq!(Field::parse("body."), None);
        assert_eq!(Field::parse("body.A/B"), None);
        assert_eq!(Field::parse("apikey"), None, "the case is the wire's");
        assert_eq!(Field::parse("body.KEY"), Some(Field::Body("KEY".into())));
        assert_eq!(Field::Body("KEY".into()).to_string(), "body.KEY");
        for field in [
            Field::Password,
            Field::Otp,
            Field::ApiKey,
            Field::PrivateKey,
            Field::Body("K".into()),
            Field::Number,
            Field::Cvc,
        ] {
            assert_eq!(Field::parse(&field.to_string()), Some(field.clone()));
            for kind in field.kinds() {
                assert!(
                    fields_for(kind).contains(&field.to_string().split('.').next().unwrap()),
                    "{field} on {kind}"
                );
            }
        }
    }

    #[test]
    fn the_fields_of_a_kind_are_what_a_grant_could_cover() {
        assert_eq!(fields_for("login"), ["password", "otp"]);
        assert_eq!(fields_for("apikey"), ["apiKey"]);
        assert_eq!(fields_for("ssh"), ["privateKey"]);
        assert_eq!(fields_for("card"), ["number", "cvc"]);
        assert_eq!(fields_for("env"), ["body"]);
        assert!(fields_for("note").is_empty());
        assert!(fields_for("identity").is_empty());
        assert!(fields_for("").is_empty());
    }

    #[test]
    fn a_slug_is_the_title_in_kebab_case() {
        assert_eq!(slug("Stripe (live)"), "stripe-live");
        assert_eq!(slug("GitHub"), "github");
        assert_eq!(slug("  --AWS -- prod__key!  "), "aws-prod-key");
        assert_eq!(slug("already-a-slug"), "already-a-slug");
        assert_eq!(slug("Ключ API"), "ключ-api");
        assert_eq!(slug("***"), "");
        assert_eq!(slug(""), "");
    }
}
