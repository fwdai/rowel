//! Finding the owner's other devices on the local network (design §5,
//! "Discovery"). Each device advertises the service [`SERVICE_TYPE`] with its
//! listener's port and a TXT record ([`Txt`]), and browses for the others'.
//!
//! A TXT says something only to a device that holds one of the same lockbox
//! keys: a random nonce, new every session, and for every lockbox key the
//! device holds (current and retired) a tag and the device's id masked under
//! that key (`trust::discovery_tag`, `trust::mask_device_id`). A holder
//! recognises the tag and reads the id; anyone else sees a nonce and noise
//! that links nothing to the last session.
//!
//! Behind the [`Discovery`] trait, so the session logic is tested against the
//! in-memory network ([`memory`]); the app's own is [`mdns`].

use std::net::SocketAddr;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;

use super::channel::PROTOCOL_VERSION;
use super::trust::{NONCE_LEN, TAG_LEN};
use super::Result;

pub mod mdns;
#[cfg(test)]
pub mod memory;

/// The DNS-SD service type, with mDNS's domain.
pub const SERVICE_TYPE: &str = "_rowel-sync._tcp.local.";

/// The most lockbox keys one advertisement carries. Each is a property of
/// about sixty bytes, and the whole record has to fit one packet.
pub const MAX_TAGS: usize = 16;

/// One lockbox key in an advertisement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tag {
    /// `trust::discovery_tag` of the key over the advertisement's nonce.
    pub tag: [u8; TAG_LEN],
    /// The advertiser's device id, masked under the same key
    /// (`trust::mask_device_id`).
    pub device: [u8; 32],
}

/// What an advertisement says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Txt {
    pub nonce: [u8; NONCE_LEN],
    pub tags: Vec<Tag>,
}

impl Txt {
    /// The record as TXT properties: `v` the protocol version, `n` the nonce
    /// in hex, and `t0`… one tag each (the tag and the masked id, base64url).
    /// Tags past [`MAX_TAGS`] are left out.
    pub fn properties(&self) -> Vec<(String, String)> {
        let mut props = vec![
            ("v".to_string(), PROTOCOL_VERSION.to_string()),
            ("n".to_string(), hex::encode(self.nonce)),
        ];
        for (i, tag) in self.tags.iter().take(MAX_TAGS).enumerate() {
            let mut bytes = [0u8; TAG_LEN + 32];
            bytes[..TAG_LEN].copy_from_slice(&tag.tag);
            bytes[TAG_LEN..].copy_from_slice(&tag.device);
            props.push((format!("t{i}"), URL_SAFE_NO_PAD.encode(bytes)));
        }
        props
    }

    /// Read a record back from its properties. `None` for one of another
    /// protocol version or without a well-formed nonce: a device this build
    /// cannot speak to. A tag that is not well formed, or past [`MAX_TAGS`],
    /// is skipped; the rest still counts.
    pub fn parse<'a>(props: impl IntoIterator<Item = (&'a str, &'a str)>) -> Option<Self> {
        let mut version = None;
        let mut nonce = None;
        let mut tags = Vec::new();
        for (key, value) in props {
            match key {
                "v" => version = Some(value),
                "n" => {
                    let mut bytes = [0u8; NONCE_LEN];
                    if hex::decode_to_slice(value, &mut bytes).is_ok() {
                        nonce = Some(bytes);
                    }
                }
                _ => {
                    if let Some(tag) = tag_property(key, value) {
                        tags.push(tag);
                    }
                }
            }
        }
        if version != Some(PROTOCOL_VERSION.to_string().as_str()) {
            return None;
        }
        Some(Self {
            nonce: nonce?,
            tags,
        })
    }
}

fn tag_property(key: &str, value: &str) -> Option<Tag> {
    let index: usize = key.strip_prefix('t')?.parse().ok()?;
    if index >= MAX_TAGS {
        return None;
    }
    let bytes = URL_SAFE_NO_PAD.decode(value).ok()?;
    if bytes.len() != TAG_LEN + 32 {
        return None;
    }
    Some(Tag {
        tag: bytes[..TAG_LEN].try_into().ok()?,
        device: bytes[TAG_LEN..].try_into().ok()?,
    })
}

/// This device's advertisement: where its listener is, and what it says.
#[derive(Clone, Debug)]
pub struct Advert {
    pub port: u16,
    pub txt: Txt,
}

/// Another device's advertisement, as seen.
#[derive(Clone, Debug)]
pub struct Peer {
    /// The advertisement's instance name: random per session, and what a
    /// later [`Event::Gone`] names it by.
    pub instance: String,
    pub txt: Txt,
    /// Where its listener is.
    pub addrs: Vec<SocketAddr>,
}

pub enum Event {
    /// An advertisement appeared or changed.
    Seen(Peer),
    /// The advertisement under `instance` went away.
    Gone { instance: String },
}

/// Where a [`Discovery`] reports what it sees, on a thread of its own.
pub type OnEvent = Box<dyn Fn(Event) + Send + Sync>;

/// Advertising this device and browsing for the others.
pub trait Discovery: Send + Sync {
    /// Advertise `advert` in place of whatever was advertised before.
    fn advertise(&self, advert: &Advert) -> Result<()>;
    /// Report every other device's advertisement to `on_event`, seen and
    /// gone, until [`Discovery::stop`].
    fn browse(&self, on_event: OnEvent) -> Result<()>;
    /// Take the advertisement down and stop browsing. Nothing is reported
    /// afterwards.
    fn stop(&self);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn txt() -> Txt {
        Txt {
            nonce: [7; NONCE_LEN],
            tags: vec![
                Tag {
                    tag: [1; TAG_LEN],
                    device: [2; 32],
                },
                Tag {
                    tag: [3; TAG_LEN],
                    device: [4; 32],
                },
            ],
        }
    }

    fn parse(props: &[(String, String)]) -> Option<Txt> {
        Txt::parse(props.iter().map(|(k, v)| (k.as_str(), v.as_str())))
    }

    #[test]
    fn a_record_reads_back_as_written() {
        assert_eq!(parse(&txt().properties()), Some(txt()));
    }

    // The nonce and the tags are all a record says, and they are bytes: no
    // name and no id in the clear.
    #[test]
    fn a_record_carries_only_the_version_the_nonce_and_the_tags() {
        let keys: Vec<String> = txt().properties().into_iter().map(|(k, _)| k).collect();
        assert_eq!(keys, ["v", "n", "t0", "t1"]);
    }

    #[test]
    fn another_version_or_a_missing_nonce_is_no_record() {
        let mut props = txt().properties();
        props[0].1 = (PROTOCOL_VERSION + 1).to_string();
        assert_eq!(parse(&props), None);
        let props: Vec<_> = txt()
            .properties()
            .into_iter()
            .filter(|(k, _)| k != "n")
            .collect();
        assert_eq!(parse(&props), None);
        let mut props = txt().properties();
        props[1].1 = "zz".into();
        assert_eq!(parse(&props), None);
    }

    #[test]
    fn a_malformed_tag_is_skipped_and_the_rest_still_count() {
        let mut props = txt().properties();
        props[2].1 = "not base64!".into();
        props.push((format!("t{MAX_TAGS}"), props[3].1.clone()));
        let read = parse(&props).unwrap();
        assert_eq!(read.tags, &txt().tags[1..]);
    }

    #[test]
    fn a_record_carries_at_most_max_tags() {
        let many = Txt {
            tags: vec![
                Tag {
                    tag: [1; TAG_LEN],
                    device: [2; 32]
                };
                MAX_TAGS + 4
            ],
            ..txt()
        };
        let props = many.properties();
        assert_eq!(parse(&props).unwrap().tags.len(), MAX_TAGS);
        // Every property fits a TXT string's single length byte.
        assert!(props.iter().all(|(k, v)| k.len() + 1 + v.len() <= 255));
    }
}
