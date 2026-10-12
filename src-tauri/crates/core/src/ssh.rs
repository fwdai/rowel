//! SSH private keys as the `ssh` kind holds them: an OpenSSH block
//! (`-----BEGIN OPENSSH PRIVATE KEY-----`), the form `ssh-keygen` writes and
//! every OpenSSH client reads. An exporter may hand us a key in PKCS#8
//! (`-----BEGIN PRIVATE KEY-----`) instead — 1Password's does — and OpenSSH
//! reads that form for RSA and ECDSA keys but not for Ed25519, which is what
//! most such keys are (`ssh-keygen -y` on one says "invalid format"). So a
//! PKCS#8 key is converted on the way in, and its public line and fingerprint
//! come with it, as the generator derives them for a key made here.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use p256::elliptic_curve::sec1::ToEncodedPoint;
use p256::pkcs8::DecodePrivateKey;
use ssh_key::private::{EcdsaKeypair, Ed25519Keypair, KeypairData, RsaKeypair};
use ssh_key::{HashAlg, LineEnding, PrivateKey};

pub const OPENSSH_HEADER: &str = "-----BEGIN OPENSSH PRIVATE KEY-----";
const PKCS8_HEADER: &str = "-----BEGIN PRIVATE KEY-----";
const PKCS8_FOOTER: &str = "-----END PRIVATE KEY-----";

/// A key in the form the `ssh` kind holds, with what is derived from it: the
/// one-line public key and its `SHA256:…` fingerprint.
#[derive(Debug)]
pub struct OpenSsh {
    pub private_key: String,
    pub public_key: String,
    pub fingerprint: String,
}

pub fn is_openssh(block: &str) -> bool {
    block.trim_start().starts_with(OPENSSH_HEADER)
}

/// The plain PKCS#8 form only: an encrypted one (`BEGIN ENCRYPTED PRIVATE
/// KEY`) needs a passphrase this has no way to ask for.
pub fn is_pkcs8(block: &str) -> bool {
    block.trim_start().starts_with(PKCS8_HEADER)
}

/// A PKCS#8 PEM block as an OpenSSH one. `Err` names what stood in the way:
/// a block that is not plain PKCS#8, one that does not decode, or a key of an
/// algorithm OpenSSH has no name for.
pub fn from_pkcs8(pem: &str) -> Result<OpenSsh, String> {
    let der = pkcs8_der(pem)?;
    let key = PrivateKey::new(keypair(&der)?, "").map_err(|e| e.to_string())?;
    Ok(OpenSsh {
        private_key: key
            .to_openssh(LineEnding::LF)
            .map_err(|e| e.to_string())?
            .to_string(),
        public_key: key.public_key().to_openssh().map_err(|e| e.to_string())?,
        fingerprint: key.fingerprint(HashAlg::Sha256).to_string(),
    })
}

fn pkcs8_der(pem: &str) -> Result<Vec<u8>, String> {
    let body = pem
        .trim()
        .strip_prefix(PKCS8_HEADER)
        .ok_or("not a plain PKCS#8 private key")?
        .strip_suffix(PKCS8_FOOTER)
        .ok_or("PKCS#8 block has no end line")?;
    let body: String = body.chars().filter(|c| !c.is_whitespace()).collect();
    STANDARD
        .decode(body)
        .map_err(|e| format!("PKCS#8 block is not base64: {e}"))
}

// The algorithm is named inside the DER, and each decoder refuses every
// other's, so they are tried in turn — Ed25519 first, being what most keys
// are. The public point of an ECDSA key is derived here; OpenSSH's form
// carries it beside the scalar, where PKCS#8 may leave it out.
fn keypair(der: &[u8]) -> Result<KeypairData, String> {
    if let Ok(key) = ed25519_dalek::SigningKey::from_pkcs8_der(der) {
        return Ok(Ed25519Keypair::from(key).into());
    }
    if let Ok(key) = rsa::RsaPrivateKey::from_pkcs8_der(der) {
        return RsaKeypair::try_from(key)
            .map(Into::into)
            .map_err(|e| e.to_string());
    }
    if let Ok(key) = p256::SecretKey::from_pkcs8_der(der) {
        return Ok(EcdsaKeypair::NistP256 {
            public: key.public_key().to_encoded_point(false),
            private: key.into(),
        }
        .into());
    }
    if let Ok(key) = p384::SecretKey::from_pkcs8_der(der) {
        return Ok(EcdsaKeypair::NistP384 {
            public: key.public_key().to_encoded_point(false),
            private: key.into(),
        }
        .into());
    }
    if let Ok(key) = p521::SecretKey::from_pkcs8_der(der) {
        return Ok(EcdsaKeypair::NistP521 {
            public: key.public_key().to_encoded_point(false),
            private: key.into(),
        }
        .into());
    }
    Err("not an Ed25519, RSA or ECDSA key, the kinds OpenSSH names".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Three keys `openssl genpkey` wrote, with the public line and fingerprint
    // `ssh-keygen -y` / `-l` print for the RSA and ECDSA ones (OpenSSH reads
    // those two in PKCS#8) and, for the Ed25519 one it refuses, the same two
    // computed by hand from the seed.
    const ED25519: &str = "-----BEGIN PRIVATE KEY-----\n\
MC4CAQAwBQYDK2VwBCIEIFYAdBe0wA2kXej4h4SX8AGsC/kjI4TKWEcivS/EC8Rb\n\
-----END PRIVATE KEY-----\n";
    const RSA: &str = "-----BEGIN PRIVATE KEY-----\n\
MIICdwIBADANBgkqhkiG9w0BAQEFAASCAmEwggJdAgEAAoGBAJjWyAx6sX12daFL\n\
RdPfZCYH4DnByVLTbve5qhH5Z52mbv2nZEu2/XLRZNTrEBkVvZi9BBAlV7l+BtBd\n\
QriVh81f3doF7d6RLrf2SRXYL0BsUb0kpVtlVPbFI7PNK21KdriVTrWQGe0dxF4l\n\
9Vy3kAEuvwCWtnkslGchDAZWAN+XAgMBAAECgYBvliBoPAvp9drb8AEDN03/FjD2\n\
+izqc8PIyZ0JknW7/A980gG/5dIVShu7+ALAzcanBRk6UIlDvGdtKSnVahbDEhyd\n\
W3N8fBZhDyuJgrj0UlxAVpTSe+tUs/LpdWFUAdmn2KF4PEM9APopd4QUQ6ziTRLt\n\
f3PeAYkP0WtFPcB2wQJBAMoFDZ8zMgcZW5VdKVGRe+WuqDjUrDTTJufI/0aggWgr\n\
iM50SRxDXS91X526oemWpi/It1MTFnJvWP1We/uyzV0CQQDBrZIwn2jJKJgkwofS\n\
UfhXd0fXMx+rcltnWccDuWWumeQMJJhhM1nDd9CCw4xSnKgdc7nHSl7jihqf3KBD\n\
E12DAkByZ64xLBL2xjuBOML/eH5Lsj99/6KOP83WkZNPcfIXBiBxmpUUtml9SOEf\n\
N+Qdnrz6bpazKyoZIEmrHmEUR0vZAkEAhhmpDBOD2SxLU8CBjGWHHwCO9HkIZ4wx\n\
r3ig7Ho05lTVanb4Lu0ALPBLpg7BnP2tBV94dX2P/9pCnweCy46gAwJBAI8CygC3\n\
KKHGU8sI153PXI7ddt0GyRP0r2++3QRexVOVmO9jkD3BZpBDN46BObl5F8JZCeQf\n\
qpISp+duEkL5pHQ=\n\
-----END PRIVATE KEY-----\n";
    const P256: &str = "-----BEGIN PRIVATE KEY-----\n\
MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgmdsrNmrHD+bXtAlW\n\
Bb3grGSmiOMl2J8CAubRjiL+Z02hRANCAATqkbBP9fv7N9vFsnz9IcoSuZpDuJ7g\n\
aaCAATJ9YwXsBBP9K3kLDy3+474w67jM8vIp310yl3WZlyCXLreyk0cp\n\
-----END PRIVATE KEY-----\n";

    fn convert(pem: &str) -> OpenSsh {
        let key = from_pkcs8(pem).unwrap();
        assert!(is_openssh(&key.private_key));
        // What came out is a key OpenSSH's own reader takes back, and the
        // same key: its public half matches the line beside it.
        let back = PrivateKey::from_openssh(&key.private_key).unwrap();
        assert_eq!(back.public_key().to_openssh().unwrap(), key.public_key);
        key
    }

    #[test]
    fn an_ed25519_pkcs8_key_becomes_an_openssh_one() {
        let key = convert(ED25519);
        assert_eq!(
            key.public_key,
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJWRyIiw75kU1Wo8Pmls6ZtTKfHn58UsSI84WIrPiVNB"
        );
        assert_eq!(
            key.fingerprint,
            "SHA256:NVYSMnDilri+HeSJWIVb9W6V043z6EzNZcY3Tp0hVQo"
        );
    }

    #[test]
    fn an_rsa_pkcs8_key_becomes_an_openssh_one() {
        let key = convert(RSA);
        assert_eq!(
            key.public_key,
            "ssh-rsa AAAAB3NzaC1yc2EAAAADAQABAAAAgQCY1sgMerF9dnWhS0XT32QmB+A5wclS0273uaoR+Wedpm79p2RLtv1y0WTU6xAZFb2YvQQQJVe5fgbQXUK4lYfNX93aBe3ekS639kkV2C9AbFG9JKVbZVT2xSOzzSttSna4lU61kBntHcReJfVct5ABLr8AlrZ5LJRnIQwGVgDflw=="
        );
        assert_eq!(
            key.fingerprint,
            "SHA256:uQp0yWUUFGn6QkJMYHDqsJ0vClGrTUmNw8s5oz+oXm4"
        );
    }

    #[test]
    fn a_p256_pkcs8_key_becomes_an_openssh_one() {
        let key = convert(P256);
        assert_eq!(
            key.public_key,
            "ecdsa-sha2-nistp256 AAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAAIbmlzdHAyNTYAAABBBOqRsE/1+/s328WyfP0hyhK5mkO4nuBpoIABMn1jBewEE/0reQsPLf7jvjDruMzy8infXTKXdZmXIJcut7KTRyk="
        );
        assert_eq!(
            key.fingerprint,
            "SHA256:Ge1rTgao03CAcoL2itzexiH5RdOQ8cdC1mpqt6Z6rR0"
        );
    }

    // What is not a plain PKCS#8 key is named for what it is, not guessed at.
    #[test]
    fn anything_but_a_plain_pkcs8_block_is_refused_by_name() {
        let openssh = convert(ED25519).private_key;
        assert_eq!(
            from_pkcs8(&openssh).unwrap_err(),
            "not a plain PKCS#8 private key"
        );
        assert!(!is_pkcs8(&openssh) && !is_pkcs8("-----BEGIN ENCRYPTED PRIVATE KEY-----"));
        assert_eq!(
            from_pkcs8("-----BEGIN PRIVATE KEY-----\nabc").unwrap_err(),
            "PKCS#8 block has no end line"
        );
        assert!(
            from_pkcs8("-----BEGIN PRIVATE KEY-----\n@@@\n-----END PRIVATE KEY-----")
                .unwrap_err()
                .starts_with("PKCS#8 block is not base64")
        );
        // Valid base64, but no key of any kind inside.
        assert_eq!(
            from_pkcs8("-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----").unwrap_err(),
            "not an Ed25519, RSA or ECDSA key, the kinds OpenSSH names"
        );
    }
}
