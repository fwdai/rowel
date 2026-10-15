//! SSH private keys as the `ssh` kind holds them: an OpenSSH block
//! (`-----BEGIN OPENSSH PRIVATE KEY-----`), the form `ssh-keygen` writes and
//! every OpenSSH client reads. An exporter may hand us a key in PKCS#8
//! (`-----BEGIN PRIVATE KEY-----`) instead — 1Password's does — and OpenSSH
//! reads that form for RSA and ECDSA keys but not for Ed25519, which is what
//! most such keys are (`ssh-keygen -y` on one says "invalid format"). So a
//! PKCS#8 key is converted on the way in, and its public line and fingerprint
//! come with it, as the generator derives them for a key made here.
//!
//! An RSA key is the one left as it came: OpenSSH reads it in PKCS#8 form,
//! and the one Rust crate that would decode it carries an open timing
//! advisory (RUSTSEC-2023-0071) that keeps it out of the build.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use p256::elliptic_curve::sec1::ToEncodedPoint;
use p256::pkcs8::{DecodePrivateKey, ObjectIdentifier, PrivateKeyInfo};
use ssh_key::private::{EcdsaKeypair, Ed25519Keypair, KeypairData};
use ssh_key::{HashAlg, LineEnding, PrivateKey, PublicKey};

pub const OPENSSH_HEADER: &str = "-----BEGIN OPENSSH PRIVATE KEY-----";
const PKCS8_HEADER: &str = "-----BEGIN PRIVATE KEY-----";
const PKCS8_FOOTER: &str = "-----END PRIVATE KEY-----";
/// `rsaEncryption`, the algorithm a PKCS#8 RSA key names.
const RSA_OID: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.1");

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

/// A PKCS#8 PEM block as an OpenSSH one. `Ok(None)` is an RSA key, which
/// stays as it came (see the module note). `Err` names what stood in the
/// way: a block that is not plain PKCS#8, one that does not decode, or a key
/// of an algorithm OpenSSH has no name for.
pub fn from_pkcs8(pem: &str) -> Result<Option<OpenSsh>, String> {
    let der = pkcs8_der(pem)?;
    let Some(data) = keypair(&der)? else {
        return Ok(None);
    };
    let key = PrivateKey::new(data, "").map_err(|e| e.to_string())?;
    Ok(Some(OpenSsh {
        private_key: key
            .to_openssh(LineEnding::LF)
            .map_err(|e| e.to_string())?
            .to_string(),
        public_key: key.public_key().to_openssh().map_err(|e| e.to_string())?,
        fingerprint: key.fingerprint(HashAlg::Sha256).to_string(),
    }))
}

/// What a key says about itself: its one-line public key and its `SHA256:…`
/// fingerprint, as `ssh-keygen -y` and `ssh-keygen -l` print them.
///
/// Read from the OpenSSH private block first, since that is the key an entry
/// holds, and from the public line when the block is in another form (a
/// PKCS#8 RSA key, say) or missing. An encrypted block needs no passphrase
/// here: OpenSSH keeps a key's public half in the clear. `None` when neither
/// parses, so the caller leaves alone whatever the entry already carries.
pub fn public_of(private_key: Option<&str>, public_key: Option<&str>) -> Option<(String, String)> {
    let key = private_key
        .filter(|block| is_openssh(block))
        .and_then(|block| PrivateKey::from_openssh(block.trim()).ok())
        .map(|key| key.public_key().clone())
        .or_else(|| public_key.and_then(|line| PublicKey::from_openssh(line.trim()).ok()))?;
    let line = key.to_openssh().ok()?;
    Some((line, key.fingerprint(HashAlg::Sha256).to_string()))
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
// carries it beside the scalar, where PKCS#8 may leave it out. `None` is an
// RSA key, told by its algorithm's own id, since nothing here decodes one.
fn keypair(der: &[u8]) -> Result<Option<KeypairData>, String> {
    let info = PrivateKeyInfo::try_from(der).map_err(|_| "not a PKCS#8 key".to_owned())?;
    if info.algorithm.oid == RSA_OID {
        return Ok(None);
    }
    if let Ok(key) = ed25519_dalek::SigningKey::from_pkcs8_der(der) {
        return Ok(Some(Ed25519Keypair::from(key).into()));
    }
    if let Ok(key) = p256::SecretKey::from_pkcs8_der(der) {
        return Ok(Some(
            EcdsaKeypair::NistP256 {
                public: key.public_key().to_encoded_point(false),
                private: key.into(),
            }
            .into(),
        ));
    }
    if let Ok(key) = p384::SecretKey::from_pkcs8_der(der) {
        return Ok(Some(
            EcdsaKeypair::NistP384 {
                public: key.public_key().to_encoded_point(false),
                private: key.into(),
            }
            .into(),
        ));
    }
    if let Ok(key) = p521::SecretKey::from_pkcs8_der(der) {
        return Ok(Some(
            EcdsaKeypair::NistP521 {
                public: key.public_key().to_encoded_point(false),
                private: key.into(),
            }
            .into(),
        ));
    }
    Err("not an Ed25519 or ECDSA key".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Three keys `openssl genpkey` wrote, with the public line and fingerprint
    // `ssh-keygen -y` / `-l` print for the ECDSA one (OpenSSH reads it, and
    // the RSA one, in PKCS#8) and, for the Ed25519 one it refuses, the same
    // two computed by hand from the seed.
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
        let key = from_pkcs8(pem).unwrap().unwrap();
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

    // OpenSSH reads it as it is, and the crate that would convert it is kept
    // out of the build: an RSA key is recognised and left alone, not refused.
    #[test]
    fn an_rsa_pkcs8_key_is_left_as_it_came() {
        assert!(from_pkcs8(RSA).unwrap().is_none());
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
            "not a PKCS#8 key"
        );
    }

    // Throwaway keys `ssh-keygen` wrote for these tests, with the public line
    // it wrote beside each and the fingerprint `ssh-keygen -l -E sha256`
    // printed for it.
    const OPENSSH_ED25519: &str = "-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW
QyNTUxOQAAACCMsBkG+92FeO1IQ4QRB95kBxqhxtJBCCMO5vK3bPfY0QAAAJAWWxCmFlsQ
pgAAAAtzc2gtZWQyNTUxOQAAACCMsBkG+92FeO1IQ4QRB95kBxqhxtJBCCMO5vK3bPfY0Q
AAAECsxtAPvmVXd9tYi9tmkGVfy+u9tAH5TC3ZeXz9Ic6g4IywGQb73YV47UhDhBEH3mQH
GqHG0kEIIw7m8rds99jRAAAAC3BsYWluQHJvd2VsAQI=
-----END OPENSSH PRIVATE KEY-----
";
    const OPENSSH_ED25519_PUBLIC: &str =
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIIywGQb73YV47UhDhBEH3mQHGqHG0kEIIw7m8rds99jR plain@rowel";
    const OPENSSH_ED25519_FINGERPRINT: &str = "SHA256:MvV+b7yWsyADqckWJezjL66WS1klaUagTWUU4Svymg4";

    // Protected with the passphrase "hunter2", which nothing here is given.
    const OPENSSH_ENCRYPTED: &str = "-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAACmFlczI1Ni1jdHIAAAAGYmNyeXB0AAAAGAAAABCDtOmhQu
Z7zkATqwvAOzmTAAAAGAAAAAEAAAAzAAAAC3NzaC1lZDI1NTE5AAAAIBaV7k4YmJ+7kTHZ
/Ih8Noc5QrKCu+Aw+iN38JvW4tIkAAAAkFcAXXyZXaIBk+QtQ6/iRSFXcE/M9rDFHwOCmW
oy0ozpOVx5fHWZN+W2j0eW3OT3SYIYIPZ+sHUwDScXS7GgFEij+d7zZ6MoW9Q4nv37miHY
Ysz9hhTh42lCoXwZ9ScbKbP2LVAKfXe7uKvFlCVDIeIcN7eU2NEJ9xB6cWmXtV9DZtqB5k
47NekuCCdEOaAIcw==
-----END OPENSSH PRIVATE KEY-----
";

    const OPENSSH_RSA: &str = "-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAlwAAAAdzc2gtcn
NhAAAAAwEAAQAAAIEAxKHe2s9qukhhQ0/EI8e/MqwDm+o9IPCJ4+QCb1NnJ23LHP9UUdeM
wBqbyNcGMNHc9Lm2YOLABE2SLSGHgskFIeiApyr0ZlviRqgYZxsSreWnbBOcb41jtkvSbQ
fQ+M8Uk49zO7DMs2lgLl9AGs9dBTNgVTgT9ve3WnSQMNRWNIkAAAIACybC4AsmwuAAAAAH
c3NoLXJzYQAAAIEAxKHe2s9qukhhQ0/EI8e/MqwDm+o9IPCJ4+QCb1NnJ23LHP9UUdeMwB
qbyNcGMNHc9Lm2YOLABE2SLSGHgskFIeiApyr0ZlviRqgYZxsSreWnbBOcb41jtkvSbQfQ
+M8Uk49zO7DMs2lgLl9AGs9dBTNgVTgT9ve3WnSQMNRWNIkAAAADAQABAAAAgQCdQ9m5p6
r00m74JxP4igmhvXIlgXLipX+jhPltkIcwQd/xkUUfMAainqV6PH/3C+Js2RvE4ZYx2tjO
v8JiF+TBkwIzwQjVNtPoJSXibMla8lro7TfC4yjYQ8PbdcW1rFnp+n3wmn1vseFp4kv87+
uM6xOCkIEvjlv7MDnuCcu/AQAAAEAH3dPZGPgQ6eBc474hz/oUdfFsJKDWnlpQw4wfjI9p
rsExEZHICi1KDq4Ap5V1epaGvY+SFk+yZ3b6QLIEjG1MAAAAQQDkNVUeLQeXC9wSqjGfy9
53mIxcq8F7yxZJm/8vaWuDn5zv8ockGd5fFxl1rOKIYxJ7EEUM78W41xE36Yd58qh3AAAA
QQDclB4lfS5SO9o1FqaLARFniyVhInUMYaJKl8zmTvVzY/mmLzmWeeTAHw+UmpGxrgHs+t
HKOXJ/heenSu3KAkr/AAAACXJzYUByb3dlbAE=
-----END OPENSSH PRIVATE KEY-----
";

    const OPENSSH_ECDSA_PUBLIC: &str = "ecdsa-sha2-nistp256 AAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAAIbmlzdHAyNTYAAABBBK86MNn4rWjhQzHkceG+MUbCvIYo8rFKQv22wR09RENj1S+0QOJFj6rQCIBpLbBLbUBbq1QvdTmrdiPVkPQt4kA= ec@rowel";

    #[test]
    fn a_pasted_private_block_gives_its_public_line_and_fingerprint() {
        let (line, fingerprint) = public_of(Some(OPENSSH_ED25519), None).unwrap();
        assert_eq!(line, OPENSSH_ED25519_PUBLIC);
        assert_eq!(fingerprint, OPENSSH_ED25519_FINGERPRINT);
    }

    #[test]
    fn an_encrypted_block_gives_its_fingerprint_without_the_passphrase() {
        let (line, fingerprint) = public_of(Some(OPENSSH_ENCRYPTED), None).unwrap();
        assert!(line.starts_with(
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBaV7k4YmJ+7kTHZ/Ih8Noc5QrKCu+Aw+iN38JvW4tIk"
        ));
        assert_eq!(
            fingerprint,
            "SHA256:j2Jamcs+CkEep3CunOmyaNg7fVBdxKc+rjIq3msTbYM"
        );
    }

    #[test]
    fn an_rsa_block_is_read_as_well() {
        let (_, fingerprint) = public_of(Some(OPENSSH_RSA), None).unwrap();
        assert_eq!(
            fingerprint,
            "SHA256:yvINds+sIjMNn2Mf8Lzd4aU8MFl+Mp6C/UaT5pZ0JZo"
        );
    }

    // The private block is the key the entry holds, so it wins over a public
    // line that names some other key; with no block that parses, the line is
    // what there is.
    #[test]
    fn the_private_block_wins_and_the_public_line_stands_in_for_it() {
        let (_, fingerprint) =
            public_of(Some(OPENSSH_ED25519), Some(OPENSSH_ECDSA_PUBLIC)).unwrap();
        assert_eq!(fingerprint, OPENSSH_ED25519_FINGERPRINT);

        let (line, fingerprint) = public_of(Some(RSA), Some(OPENSSH_ECDSA_PUBLIC)).unwrap();
        assert_eq!(line, OPENSSH_ECDSA_PUBLIC);
        assert_eq!(
            fingerprint,
            "SHA256:IGlniK29M31+RlxsarcPQhepK09xmUGe/RNs/uM9gNQ"
        );
    }

    #[test]
    fn an_ssh_entry_is_completed_from_its_key_and_nothing_else_is_touched() {
        use crate::models::Entry;
        let ssh = |public: Option<&str>, fingerprint: Option<&str>| Entry {
            kind: "ssh".into(),
            private_key: Some(OPENSSH_ED25519.into()),
            public_key: public.map(Into::into),
            fingerprint: fingerprint.map(Into::into),
            ..Default::default()
        };

        // Pasted on its own: both are filled in.
        let mut pasted = ssh(None, None);
        pasted.derive_ssh_public();
        assert_eq!(pasted.public_key.as_deref(), Some(OPENSSH_ED25519_PUBLIC));
        assert_eq!(
            pasted.fingerprint.as_deref(),
            Some(OPENSSH_ED25519_FINGERPRINT)
        );

        // A line the user typed stays; a stale fingerprint is replaced.
        let mut typed = ssh(Some("ssh-ed25519 AAAA my laptop"), Some("SHA256:stale"));
        typed.derive_ssh_public();
        assert_eq!(
            typed.public_key.as_deref(),
            Some("ssh-ed25519 AAAA my laptop")
        );
        assert_eq!(
            typed.fingerprint.as_deref(),
            Some(OPENSSH_ED25519_FINGERPRINT)
        );

        // Another kind with the same fields is not an SSH key.
        let mut note = Entry {
            kind: "note".into(),
            ..ssh(None, None)
        };
        note.derive_ssh_public();
        assert!(note.public_key.is_none() && note.fingerprint.is_none());
    }

    #[test]
    fn nothing_that_parses_gives_nothing() {
        assert!(public_of(None, None).is_none());
        assert!(public_of(Some(RSA), None).is_none());
        assert!(public_of(Some("not a key"), Some("ssh-ed25519 nope")).is_none());
    }
}
