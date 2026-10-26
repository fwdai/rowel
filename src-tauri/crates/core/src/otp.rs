//! What a stored `otp` value means.
//!
//! An entry keeps one string, and it is spelled one of two ways: a bare base32
//! seed when the code is generated with the RFC 6238 defaults, or the whole
//! `otpauth://totp/…` URI when it is not. The parameters an issuer chose are
//! part of the seed — a 8-digit or 60-second enrolment generates codes the site
//! rejects if they are dropped — so anything non-default has to survive the
//! round trip, and the compact form stays for the overwhelmingly common case.
//!
//! No Tauri or storage types here: the generator, the importers and the
//! exporters all read the same value, so the rules live in one place below
//! them. Hand-parsed rather than through `totp-rs`'s `otpauth` feature, which
//! would pull a URL/QR dependency tree in for forty lines of work. The code
//! itself is generated here too ([`code_now`]), so the app's dial, the
//! extension's `get-totp` and a headless host's `resolve` all derive it the
//! same way from the same reading of the value.

use std::fmt;
use std::ops::RangeInclusive;
use std::time::{SystemTime, UNIX_EPOCH};
use totp_rs::{Algorithm, Builder, Secret, Totp};
use url::Url;

/// The generator's defaults, and what a bare seed is understood to mean.
pub const DEFAULT_DIGITS: u32 = 6;
pub const DEFAULT_PERIOD: u64 = 30;

/// What we will generate for. Wider than any real issuer uses on purpose —
/// the point is to refuse nonsense (0 digits, a 0-second window) rather than
/// to have an opinion. Importers refuse anything outside them too, so a stored
/// value is never something `parse` would later refuse.
pub const DIGITS: RangeInclusive<u32> = 6..=10;
pub const PERIOD: RangeInclusive<u64> = 1..=300;

/// The hash a code is derived from. The three RFC 6238 names; Steam and the
/// other vendor variants are deliberately out of scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OtpAlgorithm {
    #[default]
    Sha1,
    Sha256,
    Sha512,
}

impl OtpAlgorithm {
    /// The lowercase spelling CXF writes. The URI form uppercases it, which is
    /// the convention there; both are read back case-insensitively.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Sha1 => "sha1",
            Self::Sha256 => "sha256",
            Self::Sha512 => "sha512",
        }
    }

    pub fn parse(value: &str) -> Result<Self, OtpError> {
        match value.trim().to_ascii_lowercase().as_str() {
            "sha1" => Ok(Self::Sha1),
            "sha256" => Ok(Self::Sha256),
            "sha512" => Ok(Self::Sha512),
            other => Err(OtpError(format!("unsupported otp algorithm: {other}"))),
        }
    }
}

/// Everything needed to generate a code, with the secret already normalised
/// the way the base32 decoder wants it: uppercase, unspaced, unpadded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OtpParams {
    pub secret: String,
    pub digits: u32,
    pub period: u64,
    pub algorithm: OtpAlgorithm,
}

impl Default for OtpParams {
    fn default() -> Self {
        Self {
            secret: String::new(),
            digits: DEFAULT_DIGITS,
            period: DEFAULT_PERIOD,
            algorithm: OtpAlgorithm::default(),
        }
    }
}

/// Why a value is not a usable TOTP seed. One message rather than a variant
/// per cause: every caller turns it straight into text for the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OtpError(String);

impl fmt::Display for OtpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for OtpError {}

/// The seed as the decoder wants it. Issuers print seeds in spaced groups and
/// some pad them; the backend decodes RFC4648 with padding *off*, which
/// refuses a '=' outright, so it is dropped here rather than stored and
/// rejected later.
fn secret(raw: &str) -> Result<String, OtpError> {
    let secret: String = raw
        .chars()
        .filter(|c| !c.is_whitespace())
        .map(|c| c.to_ascii_uppercase())
        .collect();
    let secret = secret.trim_end_matches('=');
    if secret.is_empty() {
        return Err(OtpError("otp secret is empty".into()));
    }
    Ok(secret.to_string())
}

/// Read a stored value in either spelling. A bare seed takes the defaults; a
/// URI takes whatever it carries, and an out-of-range parameter is an error
/// rather than a silent fallback — generating the wrong code is what locks a
/// user out, which is the whole point of this module.
pub fn parse(value: &str) -> Result<OtpParams, OtpError> {
    let value = value.trim();
    if !value
        .get(..10)
        .is_some_and(|s| s.eq_ignore_ascii_case("otpauth://"))
    {
        return Ok(OtpParams {
            secret: secret(value)?,
            ..Default::default()
        });
    }

    let uri = Url::parse(value).map_err(|e| OtpError(format!("invalid otpauth uri: {e}")))?;
    // The "host" of an otpauth URI is its type. Only `totp` is time-based; an
    // `hotp` seed run through the clock yields a plausible, wrong code every
    // time, so it is refused here rather than stored as something it is not.
    let kind = uri.host_str().unwrap_or_default();
    if !kind.eq_ignore_ascii_case("totp") {
        return Err(OtpError(format!("unsupported otp type: {kind}")));
    }
    let mut params = OtpParams::default();
    let mut seed = String::new();
    // A parameter that decides the code is read once. With two `secret`s, a
    // first-wins reader and a last-wins reader accept the same link and
    // generate different codes from it — so the field's check upstream could
    // pass a link this generator then refuses, or worse, both accept it and
    // disagree. Repeating one is an error, not a tie to break.
    let mut seen: Vec<String> = Vec::new();
    // Parameter names are not keywords, and exporters disagree about their case.
    for (key, val) in uri.query_pairs() {
        let key = key.to_ascii_lowercase();
        if matches!(key.as_str(), "secret" | "digits" | "period" | "algorithm") {
            if seen.contains(&key) {
                return Err(OtpError(format!("repeated otp parameter: {key}")));
            }
            seen.push(key.clone());
        }
        match key.as_str() {
            "secret" => seed = val.into_owned(),
            "digits" => {
                params.digits = val
                    .trim()
                    .parse()
                    .ok()
                    .filter(|d| DIGITS.contains(d))
                    .ok_or_else(|| OtpError(format!("unsupported otp digits: {val}")))?
            }
            "period" => {
                params.period = val
                    .trim()
                    .parse()
                    .ok()
                    .filter(|p| PERIOD.contains(p))
                    .ok_or_else(|| OtpError(format!("unsupported otp period: {val}")))?
            }
            "algorithm" => params.algorithm = OtpAlgorithm::parse(&val)?,
            _ => {}
        }
    }
    params.secret = secret(&seed)?;
    Ok(params)
}

/// The canonical value to store: the bare seed when there is nothing else to
/// remember, else a minimal URI. No issuer and no label — the entry already
/// carries both, and the shorter the stored string the less there is to get
/// mangled by an exporter downstream.
pub fn to_stored(params: &OtpParams) -> String {
    if params.digits == DEFAULT_DIGITS
        && params.period == DEFAULT_PERIOD
        && params.algorithm == OtpAlgorithm::default()
    {
        return params.secret.clone();
    }
    format!(
        "otpauth://totp/?secret={}&digits={}&period={}&algorithm={}",
        params.secret,
        params.digits,
        params.period,
        params.algorithm.as_str().to_ascii_uppercase()
    )
}

/// The code a stored value is good for now, with the seconds left in its
/// window and how long that window is — a dial drawn to scale needs both,
/// and 60-second seeds are not unusual.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OtpCode {
    pub code: String,
    /// Seconds left in the current window.
    pub time: u32,
    /// How long that window is — 30s unless the seed said otherwise.
    pub period: u32,
}

/// The current code for a stored `value`, in either spelling.
pub fn code_now(value: &str) -> Result<OtpCode, OtpError> {
    code_at(value, unix_now()?)
}

/// [`code_now`] at `unix_now` seconds since the Unix epoch: the step counter
/// the code is derived from, and the same reading the window countdown is
/// measured against.
pub fn code_at(value: &str, unix_now: u64) -> Result<OtpCode, OtpError> {
    let params = parse(value)?;
    let period = params.period;
    Ok(OtpCode {
        // `Token`'s Display zero-pads to the configured digit count, which is
        // exactly the string 5.x's `generate` returned.
        code: totp(&params)?.generate(unix_now).to_string(),
        time: (period - unix_now % period) as u32,
        period: period as u32,
    })
}

// The generator for one already-read stored value. The parameters are not
// optional decoration: an 8-digit or 60-second enrolment generates codes the
// site refuses if they are ignored.
fn totp(params: &OtpParams) -> Result<Totp, OtpError> {
    let secret = Secret::try_from_base32(&params.secret)
        .map_err(|e| OtpError(format!("invalid otp secret: {e:?}")))?;
    // ±1 window matches the legacy speakeasy defaults. `build_noncompliant`
    // skips the RFC secret-length and digit checks, exactly as the old
    // `new_unchecked` did: stored secrets are whatever the issuer handed the
    // user, and refusing a short one would lock them out.
    Ok(Builder::new()
        .with_algorithm(match params.algorithm {
            OtpAlgorithm::Sha1 => Algorithm::SHA1,
            OtpAlgorithm::Sha256 => Algorithm::SHA256,
            OtpAlgorithm::Sha512 => Algorithm::SHA512,
        })
        // `DIGITS` caps the count at 10, so the cast cannot truncate.
        .with_digits(params.digits as u8)
        .with_skew(1)
        .with_step_duration(params.period)
        .with_secret(secret)
        .build_noncompliant())
}

// Seconds since the Unix epoch, the clock every code is read against.
fn unix_now() -> Result<u64, OtpError> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| OtpError(e.to_string()))?
        .as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: &str = "JBSWY3DPEHPK3PXP";

    // The code has to be the one the secret is currently good for, which is
    // what the vault's own checker would accept.
    #[test]
    fn generates_the_code_for_the_current_step() {
        let result = code_now(SEED).unwrap();
        assert_eq!(result.code.len(), 6);
        assert_eq!(result.period, 30);
        assert!(result.time >= 1 && result.time <= 30);
        assert!(totp(&parse(SEED).unwrap())
            .unwrap()
            .check(&result.code, unix_now().unwrap())
            .is_some());
    }

    // The bug this guards against: the parameters on an enrolment URI were
    // dropped, so an 8-digit SHA-256 seed generated a 6-digit SHA-1 code the
    // site would never accept — silently, with no way to tell from the dial.
    #[test]
    fn honours_the_parameters_on_an_otpauth_uri() {
        let uri = "otpauth://totp/Acme:me@acme.io?secret=JBSWY3DPEHPK3PXP\
                   &digits=8&period=60&algorithm=SHA256";
        let result = code_now(uri).unwrap();
        assert_eq!(result.code.len(), 8);
        assert!(result.code.chars().all(|c| c.is_ascii_digit()));
        assert_eq!(result.period, 60);
        assert!(result.time >= 1 && result.time <= 60);
        // A 60s window is a different code from the same seed on 30s, which is
        // what "the parameters were read" actually means here.
        assert!(totp(&parse(uri).unwrap())
            .unwrap()
            .check(&result.code, unix_now().unwrap())
            .is_some());
    }

    // RFC 6238's own vectors (appendix B, the SHA-1 seed `12345678901234567890`),
    // which pin the derivation itself: the step, the digit count and the
    // zero-padding, with the window countdown measured off the same reading.
    #[test]
    fn derives_the_rfc_6238_vectors() {
        const RFC_SEED: &str = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";
        let eight = format!("otpauth://totp/x?secret={RFC_SEED}&digits=8");
        assert_eq!(
            code_at(&eight, 59).unwrap(),
            OtpCode {
                code: "94287082".into(),
                time: 1,
                period: 30,
            }
        );
        assert_eq!(code_at(&eight, 1_111_111_109).unwrap().code, "07081804");
        assert_eq!(code_at(RFC_SEED, 59).unwrap().code, "287082");
        assert_eq!(code_at(RFC_SEED, 60).unwrap().time, 30);
    }

    #[test]
    fn a_bare_secret_takes_the_defaults() {
        assert_eq!(
            parse(SEED).unwrap(),
            OtpParams {
                secret: SEED.into(),
                ..Default::default()
            }
        );
    }

    // However it was typed or pasted, one seed.
    #[test]
    fn normalises_case_spacing_and_padding() {
        for spelling in [
            "jbswy3dpehpk3pxp",
            " JBSW Y3DP EHPK 3PXP ",
            "JBSWY3DPEHPK3PXP======",
            "otpauth://totp/Acme:me@acme.io?secret=jbswy3dpehpk3pxp&issuer=Acme",
        ] {
            assert_eq!(parse(spelling).unwrap().secret, SEED, "{spelling}");
        }
    }

    #[test]
    fn only_a_totp_uri_is_accepted() {
        assert!(parse(&format!("otpauth://hotp/Acme?secret={SEED}&counter=0")).is_err());
        assert!(parse(&format!("otpauth://steam/Acme?secret={SEED}")).is_err());
        assert!(parse(&format!("otpauth://TOTP/Acme?secret={SEED}")).is_ok());
    }

    #[test]
    fn reads_every_parameter_off_a_uri() {
        let p = parse(&format!(
            "otpauth://totp/Acme:me@acme.io?secret={SEED}&digits=8&period=60&algorithm=SHA256"
        ))
        .unwrap();
        assert_eq!(p.digits, 8);
        assert_eq!(p.period, 60);
        assert_eq!(p.algorithm, OtpAlgorithm::Sha256);
    }

    // The stored form is what we parse next time, so it has to say the same
    // thing when read back.
    #[test]
    fn non_default_parameters_round_trip_through_the_stored_form() {
        let params = OtpParams {
            secret: SEED.into(),
            digits: 8,
            period: 60,
            algorithm: OtpAlgorithm::Sha512,
        };
        let stored = to_stored(&params);
        assert!(stored.starts_with("otpauth://totp/?"), "{stored}");
        assert_eq!(parse(&stored).unwrap(), params);
    }

    // The common case stays a bare seed: nothing to store means nothing to say.
    #[test]
    fn defaults_are_stored_as_the_bare_secret() {
        assert_eq!(
            to_stored(&OtpParams {
                secret: SEED.into(),
                ..Default::default()
            }),
            SEED
        );
    }

    #[test]
    fn rejects_an_empty_or_unusable_seed() {
        assert!(parse("").is_err());
        assert!(parse("   ").is_err());
        assert!(parse("otpauth://totp/Acme?issuer=Acme").is_err());
    }

    #[test]
    fn rejects_parameters_it_cannot_generate_for() {
        for bad in [
            format!("otpauth://totp/A?secret={SEED}&digits=4"),
            format!("otpauth://totp/A?secret={SEED}&digits=12"),
            format!("otpauth://totp/A?secret={SEED}&digits=six"),
            format!("otpauth://totp/A?secret={SEED}&period=0"),
            format!("otpauth://totp/A?secret={SEED}&period=99999"),
            format!("otpauth://totp/A?secret={SEED}&algorithm=md5"),
        ] {
            assert!(parse(&bad).is_err(), "{bad}");
        }
    }

    // The bug this guards against: the field checked the first `secret` and the
    // generator read the last, so a link with a good one followed by junk was
    // green in the field and empty on the dial. Whichever parameter is repeated,
    // and whether or not the copies agree, the link is refused; an `issuer`
    // twice over decides nothing and is let through.
    #[test]
    fn rejects_a_repeated_parameter_that_decides_the_code() {
        for bad in [
            format!("otpauth://totp/A?secret={SEED}&secret=NOTBASE32&digits=8"),
            format!("otpauth://totp/A?secret={SEED}&secret={SEED}"),
            format!("otpauth://totp/A?secret={SEED}&Secret={SEED}"),
            format!("otpauth://totp/A?secret={SEED}&digits=6&digits=8"),
            format!("otpauth://totp/A?secret={SEED}&period=30&period=60"),
            format!("otpauth://totp/A?secret={SEED}&algorithm=SHA1&algorithm=SHA256"),
        ] {
            assert!(parse(&bad).is_err(), "{bad}");
        }
        assert!(parse(&format!("otpauth://totp/A?secret={SEED}&issuer=A&issuer=B")).is_ok());
    }
}
