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

/// The public half of a private key, as `ssh-keygen -y` reads it: from an
/// OpenSSH block, or from a plain PKCS#8 one by way of [`from_pkcs8`]. An
/// encrypted OpenSSH block needs no passphrase here, since OpenSSH keeps a
/// key's public half in the clear.
///
/// `None` for anything this cannot read: an RSA PKCS#8 block (see the module
/// note), another format such as PuTTY's, or a block that is truncated or
/// otherwise does not decode. The caller cannot vouch for a public line
/// against such a key, and has to treat what it derived earlier as unproven.
pub fn public_half(private_key: &str) -> Option<PublicKey> {
    if is_openssh(private_key) {
        return PrivateKey::from_openssh(private_key.trim())
            .ok()
            .map(|key| key.public_key().clone());
    }
    if is_pkcs8(private_key) {
        let converted = from_pkcs8(private_key).ok()??;
        return parse_public(&converted.public_key);
    }
    None
}

/// A one-line public key (`ssh-ed25519 AAAA… comment`), or `None` when the
/// text is not one.
pub fn parse_public(line: &str) -> Option<PublicKey> {
    PublicKey::from_openssh(line.trim()).ok()
}

/// A key's `SHA256:…` fingerprint, as `ssh-keygen -l` prints it.
pub fn fingerprint(key: &PublicKey) -> String {
    key.fingerprint(HashAlg::Sha256).to_string()
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

    // The first two body lines of OPENSSH_ED25519 and its end line: what a
    // paste that lost its middle looks like.
    const OPENSSH_TRUNCATED: &str = "-----BEGIN OPENSSH PRIVATE KEY-----
b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW
QyNTUxOQAAACCMsBkG+92FeO1IQ4QRB95kBxqhxtJBCCMO5vK3bPfY0QAAAJAWWxCmFlsQ
-----END OPENSSH PRIVATE KEY-----
";

    fn line_and_print(private_key: &str) -> (String, String) {
        let key = public_half(private_key).unwrap();
        (key.to_openssh().unwrap(), fingerprint(&key))
    }

    #[test]
    fn a_private_block_gives_the_public_line_and_fingerprint_ssh_keygen_does() {
        assert_eq!(
            line_and_print(OPENSSH_ED25519),
            (
                OPENSSH_ED25519_PUBLIC.to_owned(),
                OPENSSH_ED25519_FINGERPRINT.to_owned()
            )
        );
        assert_eq!(
            line_and_print(OPENSSH_RSA).1,
            "SHA256:yvINds+sIjMNn2Mf8Lzd4aU8MFl+Mp6C/UaT5pZ0JZo"
        );
        // A PKCS#8 block is read through the same conversion an import uses.
        assert_eq!(
            line_and_print(ED25519).1,
            "SHA256:NVYSMnDilri+HeSJWIVb9W6V043z6EzNZcY3Tp0hVQo"
        );
    }

    #[test]
    fn an_encrypted_block_gives_its_public_half_without_the_passphrase() {
        let (line, print) = line_and_print(OPENSSH_ENCRYPTED);
        assert!(line.starts_with(
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIBaV7k4YmJ+7kTHZ/Ih8Noc5QrKCu+Aw+iN38JvW4tIk"
        ));
        assert_eq!(print, "SHA256:j2Jamcs+CkEep3CunOmyaNg7fVBdxKc+rjIq3msTbYM");
    }

    #[test]
    fn what_cannot_be_read_gives_nothing() {
        assert!(public_half(RSA).is_none());
        assert!(public_half(OPENSSH_TRUNCATED).is_none());
        assert!(public_half("not a key").is_none());
        assert!(parse_public("ssh-ed25519 nope").is_none());
    }

    mod entry {
        use super::*;
        use crate::models::Entry;

        fn ssh(private_key: &str, public: Option<&str>, print: Option<&str>) -> Entry {
            Entry {
                kind: "ssh".into(),
                private_key: Some(private_key.into()),
                public_key: public.map(Into::into),
                fingerprint: print.map(Into::into),
                ..Default::default()
            }
        }

        #[test]
        fn a_pasted_key_gets_its_public_line_and_fingerprint() {
            let mut pasted = ssh(OPENSSH_ED25519, None, None);
            pasted.reconcile_ssh_public(None);
            assert_eq!(pasted.public_key.as_deref(), Some(OPENSSH_ED25519_PUBLIC));
            assert_eq!(
                pasted.fingerprint.as_deref(),
                Some(OPENSSH_ED25519_FINGERPRINT)
            );
        }

        // Same key material, the user's own comment: the line is theirs to keep.
        #[test]
        fn a_line_for_the_same_key_keeps_its_comment() {
            let own = OPENSSH_ED25519_PUBLIC.replace("plain@rowel", "my laptop");
            let mut entry = ssh(OPENSSH_ED25519, Some(&own), Some("SHA256:stale"));
            entry.reconcile_ssh_public(None);
            assert_eq!(entry.public_key.as_deref(), Some(own.as_str()));
            assert_eq!(
                entry.fingerprint.as_deref(),
                Some(OPENSSH_ED25519_FINGERPRINT)
            );
        }

        // A line for any other key would be installed on a server where the
        // stored private key cannot use it, so it is replaced, comment and all.
        #[test]
        fn a_line_for_another_key_is_replaced() {
            let mut entry = ssh(OPENSSH_ED25519, Some(OPENSSH_ECDSA_PUBLIC), None);
            entry.reconcile_ssh_public(None);
            assert_eq!(entry.public_key.as_deref(), Some(OPENSSH_ED25519_PUBLIC));
            assert_eq!(
                entry.fingerprint.as_deref(),
                Some(OPENSSH_ED25519_FINGERPRINT)
            );
        }

        // An unreadable replacement cannot vouch for anything: what was
        // derived from the key it replaced goes, and is not passed off as its.
        #[test]
        fn an_unreadable_replacement_drops_what_the_old_key_left() {
            let stored = ssh(
                OPENSSH_ED25519,
                Some(OPENSSH_ED25519_PUBLIC),
                Some(OPENSSH_ED25519_FINGERPRINT),
            );
            let mut edit = ssh(
                OPENSSH_TRUNCATED,
                Some(OPENSSH_ED25519_PUBLIC),
                Some(OPENSSH_ED25519_FINGERPRINT),
            );
            edit.reconcile_ssh_public(Some(&stored));
            assert!(edit.public_key.is_none() && edit.fingerprint.is_none());

            // A line the user typed alongside it is kept, and printed.
            let mut typed = ssh(OPENSSH_TRUNCATED, Some(OPENSSH_ECDSA_PUBLIC), None);
            typed.reconcile_ssh_public(Some(&stored));
            assert_eq!(typed.public_key.as_deref(), Some(OPENSSH_ECDSA_PUBLIC));
            assert_eq!(
                typed.fingerprint.as_deref(),
                Some("SHA256:IGlniK29M31+RlxsarcPQhepK09xmUGe/RNs/uM9gNQ")
            );
        }

        // A key this cannot read, left as it was (an imported PKCS#8 RSA
        // block): nothing is proven either way, so what it came with stays.
        #[test]
        fn an_unchanged_unreadable_key_keeps_what_it_came_with() {
            let stored = ssh(RSA, None, Some("SHA256:from-the-exporter"));
            let mut edit = stored.clone();
            edit.title = "Renamed".into();
            edit.reconcile_ssh_public(Some(&stored));
            assert_eq!(
                edit.fingerprint.as_deref(),
                Some("SHA256:from-the-exporter")
            );
        }

        #[test]
        fn another_kind_is_not_touched() {
            let mut note = Entry {
                kind: "note".into(),
                ..ssh(OPENSSH_ED25519, None, None)
            };
            note.reconcile_ssh_public(None);
            assert!(note.public_key.is_none() && note.fingerprint.is_none());
        }
    }
}
