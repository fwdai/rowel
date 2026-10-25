//! The redactor: a stream with every released value replaced by its
//! placeholder (`spec/05` "Redaction"). The sidecar knows which values it
//! resolved, so whatever the child prints, or the proxy relays, passes
//! through here before the agent reads it. It is a safety net, not a
//! guarantee (Q16): the exact bytes and the obvious encodings are caught — a
//! value a program base64s, percent-encodes or JSON-escapes still reads as
//! the value — and anything transformed otherwise passes.
//!
//! The matcher streams: a chunk is scanned, everything that cannot be the
//! start of a needle is written out, and at most one byte less than the
//! longest needle is carried to the next chunk, so a value split across two
//! reads is still caught and nothing is held back longer than it must be.

use std::io::{self, Read, Write};

use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};
use base64::Engine;
use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use rowel_broker::grant::{ResolveContext, ResolveTarget};
use rowel_broker::reference::Reference;
use zeroize::Zeroizing;

use crate::agent::Broker;
use crate::authority::authority;

/// A value shorter than this is not matched at all: a needle of a few bytes
/// would redact ordinary text — a short password that is also a word — and
/// turn output to noise.
pub const MIN_LEN: usize = 6;

/// RFC 3986: everything but the unreserved characters is percent-encoded.
pub(crate) const RESERVED: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

struct Needle {
    bytes: Zeroizing<Vec<u8>>,
    placeholder: usize,
}

pub struct Redactor {
    needles: Vec<Needle>,
    placeholders: Vec<String>,
    longest: usize,
    /// The tail of the last chunk that may be the start of a needle. It may
    /// hold part of a value, so it is wiped like one.
    carry: Zeroizing<Vec<u8>>,
}

impl Redactor {
    /// Over `values`, each with the text to print in its place. Every value
    /// of [`MIN_LEN`] bytes or more is matched as itself and in standard and
    /// URL-safe base64 (padded and not), percent-encoded and JSON-escaped,
    /// when those differ from it.
    pub fn new(values: Vec<(Zeroizing<String>, String)>) -> Self {
        let mut redactor = Self {
            needles: Vec::new(),
            placeholders: Vec::new(),
            longest: 0,
            carry: Zeroizing::new(Vec::new()),
        };
        for (value, placeholder) in values {
            if value.len() < MIN_LEN {
                continue;
            }
            let index = redactor.placeholders.len();
            redactor.placeholders.push(placeholder);
            for form in encodings(&value) {
                redactor.add(form, index);
            }
        }
        redactor
    }

    fn add(&mut self, bytes: Zeroizing<Vec<u8>>, placeholder: usize) {
        if self.needles.iter().any(|n| *n.bytes == *bytes) {
            return;
        }
        self.longest = self.longest.max(bytes.len());
        self.needles.push(Needle { bytes, placeholder });
    }

    /// Whether there is anything to redact: a redactor over nothing copies.
    pub fn is_empty(&self) -> bool {
        self.needles.is_empty()
    }

    /// Scan `chunk` after what was carried over, writing out everything that
    /// is settled: text before a match, the placeholder for a match, and the
    /// tail that cannot be the start of a needle.
    pub fn feed(&mut self, chunk: &[u8], out: &mut impl Write) -> io::Result<()> {
        if self.is_empty() {
            out.write_all(chunk)?;
            return out.flush();
        }
        let mut buffer = Zeroizing::new(Vec::with_capacity(self.carry.len() + chunk.len()));
        buffer.extend_from_slice(&self.carry);
        buffer.extend_from_slice(chunk);
        self.carry.clear();

        let mut pos = 0;
        while let Some((at, len, placeholder)) = self.earliest_match(&buffer[pos..]) {
            out.write_all(&buffer[pos..pos + at])?;
            out.write_all(self.placeholders[placeholder].as_bytes())?;
            pos += at + len;
        }
        let rest = &buffer[pos..];
        let keep = self.partial_suffix(rest);
        out.write_all(&rest[..rest.len() - keep])?;
        self.carry.extend_from_slice(&rest[rest.len() - keep..]);
        out.flush()
    }

    /// Write out what was carried: the stream has ended, so it was not the
    /// start of anything.
    pub fn finish(&mut self, out: &mut impl Write) -> io::Result<()> {
        out.write_all(&self.carry)?;
        self.carry.clear();
        out.flush()
    }

    /// The first match in `hay` — the longest needle where several start at
    /// the same place — as (offset, length, placeholder index).
    fn earliest_match(&self, hay: &[u8]) -> Option<(usize, usize, usize)> {
        (0..hay.len()).find_map(|at| {
            self.needles
                .iter()
                .filter(|n| hay[at..].starts_with(&n.bytes))
                .max_by_key(|n| n.bytes.len())
                .map(|n| (at, n.bytes.len(), n.placeholder))
        })
    }

    /// How many bytes at the end of `rest` are the beginning of some needle,
    /// and so cannot be written out yet.
    fn partial_suffix(&self, rest: &[u8]) -> usize {
        let most = rest.len().min(self.longest.saturating_sub(1));
        (1..=most)
            .rev()
            .find(|&k| {
                let tail = &rest[rest.len() - k..];
                self.needles.iter().any(|n| n.bytes.starts_with(tail))
            })
            .unwrap_or(0)
    }
}

/// `rowel redact`: the redactor as a filter, stdin to stdout. Its values come
/// the only legitimate way, by resolving placeholders under a grant at the
/// place the grant names; with no placeholder it copies.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RedactArgs {
    pub grant: Option<String>,
    /// `rowel://…`, as written.
    pub refs: Vec<String>,
    /// The command line the values went to, for an `env` grant.
    pub command: Option<String>,
    /// The request the values went in, for a `header` grant.
    pub http: Option<HttpTarget>,
}

/// `--http <METHOD> <host/path>`: one concrete request, never a query string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpTarget {
    pub method: String,
    pub location: String,
}

impl HttpTarget {
    /// `host/path` as the host reads it: the host up to the first slash by
    /// the one rule (`authority`, https unless `http://` was written), the
    /// path from it (`/` when none), the method as written. A query string
    /// is refused rather than sent: it lands in logs.
    pub fn target(&self) -> Result<ResolveTarget, String> {
        let (scheme, location) = match self.location.split_once("://") {
            Some((scheme, rest)) if scheme == "https" || scheme == "http" => (scheme, rest),
            Some(_) => {
                return Err("--http takes host/path, or an https:// or http:// URL".to_string())
            }
            None => ("https", self.location.as_str()),
        };
        if location.contains(['?', '#']) {
            return Err("--http takes host/path, never a query string".to_string());
        }
        let (host, path) = match location.split_once('/') {
            Some((host, path)) => (host, format!("/{path}")),
            None => (location, "/".to_string()),
        };
        if host.is_empty() || self.method.is_empty() {
            return Err("--http takes METHOD host/path".to_string());
        }
        Ok(ResolveTarget::Http {
            host: authority(scheme, host),
            path,
            method: self.method.clone(),
        })
    }
}

/// Resolve `args.refs` under the grant and copy `input` to `output` through
/// the redactor. Exit 2 for a placeholder or target that does not read, 1
/// for a placeholder the host would not resolve, else 0.
pub fn execute(
    args: &RedactArgs,
    broker: &mut dyn Broker,
    mut input: impl Read,
    output: &mut impl Write,
    stderr: &mut impl Write,
) -> i32 {
    let mut released = Vec::new();
    if !args.refs.is_empty() {
        let target = match (&args.command, &args.http) {
            (Some(line), _) => ResolveTarget::Command(line.clone()),
            (None, Some(http)) => match http.target() {
                Ok(target) => target,
                Err(message) => {
                    let _ = writeln!(stderr, "rowel redact: {message}");
                    return 2;
                }
            },
            (None, None) => {
                let _ = writeln!(stderr, "rowel redact: --ref needs --command or --http");
                return 2;
            }
        };
        let grant = args.grant.as_deref().unwrap_or_default();
        let ctx = ResolveContext { target };
        for text in &args.refs {
            let Some(reference) = Reference::parse(text) else {
                let _ = writeln!(stderr, "rowel redact: '{text}' is not a placeholder");
                return 2;
            };
            match broker.resolve(grant, &reference, &ctx) {
                Ok(value) => released.push((value, text.clone())),
                Err(error) => {
                    let _ = writeln!(
                        stderr,
                        "rowel redact: {text} could not be resolved: {error}"
                    );
                    return 1;
                }
            }
        }
    }
    let mut redactor = Redactor::new(released);
    let mut chunk = Zeroizing::new([0u8; 8192]);
    loop {
        match input.read(&mut *chunk) {
            Ok(0) => break,
            Ok(n) => {
                if let Err(error) = redactor.feed(&chunk[..n], output) {
                    let _ = writeln!(stderr, "rowel redact: {error}");
                    return 1;
                }
            }
            Err(error) => {
                let _ = writeln!(stderr, "rowel redact: {error}");
                return 1;
            }
        }
    }
    if let Err(error) = redactor.finish(output) {
        let _ = writeln!(stderr, "rowel redact: {error}");
        return 1;
    }
    0
}

/// `value` as an `application/x-www-form-urlencoded` field carries it: like
/// percent-encoding, but a space is `+`. What the proxy writes into a form
/// body, and so one of the forms the redactor must know.
pub fn form_encoded(value: &str) -> String {
    form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

/// The forms of `value` a program may print it in, the value itself first.
fn encodings(value: &str) -> Vec<Zeroizing<Vec<u8>>> {
    let bytes = value.as_bytes();
    let mut forms = vec![Zeroizing::new(bytes.to_vec())];
    for engine in [&STANDARD, &STANDARD_NO_PAD, &URL_SAFE, &URL_SAFE_NO_PAD] {
        forms.push(Zeroizing::new(engine.encode(bytes).into_bytes()));
    }
    forms.push(Zeroizing::new(
        utf8_percent_encode(value, RESERVED)
            .to_string()
            .into_bytes(),
    ));
    forms.push(Zeroizing::new(form_encoded(value).into_bytes()));
    let json = Zeroizing::new(serde_json::to_string(value).unwrap_or_default());
    forms.push(Zeroizing::new(json.trim_matches('"').as_bytes().to_vec()));
    forms
}

#[cfg(test)]
mod tests {
    use super::*;

    fn redactor(values: &[(&str, &str)]) -> Redactor {
        Redactor::new(
            values
                .iter()
                .map(|(v, p)| (Zeroizing::new(v.to_string()), p.to_string()))
                .collect(),
        )
    }

    fn through(redactor: &mut Redactor, chunks: &[&[u8]]) -> String {
        let mut out = Vec::new();
        for chunk in chunks {
            redactor.feed(chunk, &mut out).unwrap();
        }
        redactor.finish(&mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    // Not shaped like any real provider's key: a push carrying one is refused.
    const KEY: &str = "mock-value-the-redactor-must-catch";
    const REF: &str = "rowel://lb/stripe/apiKey";

    #[test]
    fn the_exact_value_is_replaced_by_its_placeholder() {
        let mut r = redactor(&[(KEY, REF)]);
        assert_eq!(
            through(&mut r, &[format!("key={KEY}\n").as_bytes()]),
            format!("key={REF}\n")
        );
        assert_eq!(
            through(&mut r, &[format!("{KEY}{KEY}").as_bytes()]),
            format!("{REF}{REF}")
        );
    }

    #[test]
    fn each_encoding_of_the_value_is_caught() {
        let value = "p@ss/w+rd \"quoted\" \\ end";
        let mut r = redactor(&[(value, "<secret>")]);
        let forms = [
            STANDARD.encode(value),
            STANDARD_NO_PAD.encode(value),
            URL_SAFE.encode(value),
            URL_SAFE_NO_PAD.encode(value),
            utf8_percent_encode(value, RESERVED).to_string(),
            form_encoded(value),
            serde_json::to_string(value)
                .unwrap()
                .trim_matches('"')
                .to_string(),
        ];
        assert_eq!(forms[4], "p%40ss%2Fw%2Brd%20%22quoted%22%20%5C%20end");
        assert_eq!(forms[5], "p%40ss%2Fw%2Brd+%22quoted%22+%5C+end");
        assert_eq!(forms[6], "p@ss/w+rd \\\"quoted\\\" \\\\ end");
        for form in &forms {
            assert_eq!(
                through(&mut r, &[format!("[{form}]").as_bytes()]),
                "[<secret>]",
                "{form}"
            );
        }
    }

    #[test]
    fn a_value_split_across_two_chunks_is_still_caught() {
        let mut r = redactor(&[(KEY, REF)]);
        let (head, tail) = KEY.split_at(10);
        let out = through(&mut r, &[b"a ", head.as_bytes(), tail.as_bytes(), b" z"]);
        assert_eq!(out, format!("a {REF} z"));

        let mut r = redactor(&[(KEY, REF)]);
        let one_by_one: Vec<&[u8]> = KEY.as_bytes().chunks(1).collect();
        assert_eq!(through(&mut r, &one_by_one), REF);
    }

    #[test]
    fn two_values_each_get_their_own_placeholder() {
        let mut r = redactor(&[("hunter2!", "rowel://lb/gh/password"), (KEY, REF)]);
        assert_eq!(
            through(&mut r, &[format!("{KEY} and hunter2!").as_bytes()]),
            format!("{REF} and rowel://lb/gh/password")
        );
    }

    #[test]
    fn a_prefix_of_the_value_is_written_out_not_held() {
        let mut r = redactor(&[(KEY, REF)]);
        let prefix = &KEY[..12];
        let mut out = Vec::new();
        r.feed(format!("{prefix}-").as_bytes(), &mut out).unwrap();
        assert_eq!(out, format!("{prefix}-").as_bytes());
        r.feed(b"mock-v", &mut out).unwrap();
        assert_eq!(
            out,
            format!("{prefix}-").as_bytes(),
            "a possible start is carried"
        );
        r.finish(&mut out).unwrap();
        assert_eq!(out, format!("{prefix}-mock-v").as_bytes());
    }

    #[test]
    fn a_value_under_the_minimum_length_is_not_matched() {
        let mut r = redactor(&[("abc", "<short>"), ("abcdef", "<six>")]);
        assert_eq!(through(&mut r, &[b"abc abcdef abc"]), "abc <six> abc");
        assert!(redactor(&[("12345", "<five>")]).is_empty());
    }

    #[test]
    fn a_redactor_over_nothing_copies() {
        let mut r = redactor(&[]);
        assert_eq!(
            through(&mut r, &[b"anything ", b"at all"]),
            "anything at all"
        );
    }

    #[test]
    fn an_http_target_is_the_host_the_path_and_the_method() {
        let target = |method: &str, location: &str| {
            HttpTarget {
                method: method.into(),
                location: location.into(),
            }
            .target()
        };
        assert_eq!(
            target("GET", "api.stripe.com/v1/balance"),
            Ok(ResolveTarget::Http {
                host: "api.stripe.com".into(),
                path: "/v1/balance".into(),
                method: "GET".into()
            })
        );
        assert_eq!(
            target("POST", "https://api.stripe.com"),
            Ok(ResolveTarget::Http {
                host: "api.stripe.com".into(),
                path: "/".into(),
                method: "POST".into()
            })
        );
        assert_eq!(
            target("GET", "http://localhost:3000/x"),
            Ok(ResolveTarget::Http {
                host: "localhost:3000".into(),
                path: "/x".into(),
                method: "GET".into()
            })
        );
        assert_eq!(
            target("GET", "API.stripe.com:443/v1"),
            Ok(ResolveTarget::Http {
                host: "api.stripe.com".into(),
                path: "/v1".into(),
                method: "GET".into()
            })
        );
        assert!(target("GET", "api.stripe.com/v1?key=1").is_err());
        assert!(target("GET", "ftp://api.stripe.com/v1").is_err());
        assert!(target("GET", "/v1").is_err());
        assert!(target("", "api.stripe.com/v1").is_err());
    }
}
