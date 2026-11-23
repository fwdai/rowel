//! The advertisement's TXT format, and the `Discovery` behind mDNS and the tests' in-memory network.

use std::net::SocketAddr;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;

use super::channel::PROTOCOL_VERSION;
use super::trust::{NONCE_LEN, TAG_LEN};
use super::Result;

pub mod mdns;
#[cfg(test)]
pub mod memory;

pub const SERVICE_TYPE: &str = "_rowel-sync._tcp.local.";

// Each tag is a property of about sixty bytes, and the whole record has to fit one packet.
pub const MAX_TAGS: usize = 16;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tag {
    pub tag: [u8; TAG_LEN],
    pub device: [u8; 32],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Txt {
    pub nonce: [u8; NONCE_LEN],
    pub tags: Vec<Tag>,
}

impl Txt {
    pub fn properties(&self) -> Vec<(String, String)> {
        let tags = self.tags.iter().take(MAX_TAGS).enumerate().map(|(i, tag)| {
            let value = URL_SAFE_NO_PAD.encode([&tag.tag[..], &tag.device[..]].concat());
            (format!("t{i}"), value)
        });
        let head = [
            ("v".to_string(), PROTOCOL_VERSION.to_string()),
            ("n".to_string(), hex::encode(self.nonce)),
        ];
        head.into_iter().chain(tags).collect()
    }

    // None for another protocol version or no well-formed nonce; a malformed tag is skipped.
    pub fn parse<'a>(props: impl IntoIterator<Item = (&'a str, &'a str)>) -> Option<Self> {
        let (mut version, mut nonce, mut tags) = (None, None, Vec::new());
        for (key, value) in props {
            match key {
                "v" => version = Some(value),
                "n" => nonce = parse_nonce(value).or(nonce),
                _ => tags.extend(parse_tag(key, value)),
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

fn parse_nonce(value: &str) -> Option<[u8; NONCE_LEN]> {
    let mut nonce = [0u8; NONCE_LEN];
    hex::decode_to_slice(value, &mut nonce).ok()?;
    Some(nonce)
}

fn parse_tag(key: &str, value: &str) -> Option<Tag> {
    let index: usize = key.strip_prefix('t')?.parse().ok()?;
    if index >= MAX_TAGS {
        return None;
    }
    let bytes = URL_SAFE_NO_PAD.decode(value).ok()?;
    let (tag, device) = bytes.split_first_chunk()?;
    Some(Tag {
        tag: *tag,
        device: device.try_into().ok()?,
    })
}

#[derive(Clone)]
pub struct Advert {
    pub port: u16,
    pub txt: Txt,
}

pub struct Peer {
    pub instance: String,
    pub txt: Txt,
    pub addrs: Vec<SocketAddr>,
}

pub enum Event {
    Seen(Peer),
    Gone { instance: String },
}

pub type OnEvent = Box<dyn Fn(Event) + Send + Sync>;

pub trait Discovery: Send + Sync {
    // Replaces whatever was advertised before.
    fn advertise(&self, advert: &Advert) -> Result<()>;
    fn browse(&self, on_event: OnEvent) -> Result<()>;
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
