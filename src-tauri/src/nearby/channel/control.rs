//! The control messages: JSON tagged `"t": "<area>/<name>"`.

use serde::{Deserialize, Serialize};

use super::PROTOCOL_VERSION;
use crate::nearby::{Error, Result};

#[derive(Serialize, Deserialize)]
#[cfg_attr(test, derive(Debug, PartialEq, Eq))]
#[serde(tag = "t")]
pub enum Control {
    #[serde(rename = "hello/hello")]
    Hello {
        version: u32,
        name: String,
        platform: String,
    },
    #[serde(rename = "trust/proof")]
    Proof {
        vault: String,
        #[serde(with = "hex32")]
        proof: [u8; 32],
    },
    #[serde(rename = "trust/proof-retired")]
    RetiredProof {
        vault: String,
        #[serde(with = "hex32")]
        proof: [u8; 32],
    },
    #[serde(rename = "trust/done")]
    ProofsDone,
    #[serde(rename = "sync/removed")]
    Removed { ids: Vec<String> },
    #[serde(rename = "sync/state")]
    State {
        vault: String,
        revision: String,
        now_ms: i64,
    },
    #[serde(rename = "sync/pack")]
    Pack { vault: String, revision: String },
}

impl Control {
    pub fn hello(name: &str) -> Self {
        Self::Hello {
            version: PROTOCOL_VERSION,
            name: name.to_owned(),
            platform: std::env::consts::OS.to_owned(),
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            Self::Hello { .. } => "hello/hello",
            Self::Proof { .. } => "trust/proof",
            Self::RetiredProof { .. } => "trust/proof-retired",
            Self::ProofsDone => "trust/done",
            Self::Removed { .. } => "sync/removed",
            Self::State { .. } => "sync/state",
            Self::Pack { .. } => "sync/pack",
        }
    }

    pub(super) fn to_json(&self) -> Vec<u8> {
        serde_json::to_vec(self).expect("a control message serialises")
    }

    pub(super) fn from_json(bytes: &[u8]) -> Result<Self> {
        // Not serde's own message: it can quote what the peer sent.
        serde_json::from_slice(bytes)
            .map_err(|_| Error::Protocol("a malformed control message".into()))
    }
}

mod hex32 {
    use serde::de::Error as _;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8; 32], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&hex::encode(bytes))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<[u8; 32], D::Error> {
        let mut bytes = [0u8; 32];
        hex::decode_to_slice(String::deserialize(d)?, &mut bytes).map_err(D::Error::custom)?;
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nearby::channel::tests::pair;
    use crate::nearby::identity::tests::identity;

    fn one_of_each() -> [Control; 7] {
        [
            Control::hello("Laptop"),
            Control::Proof {
                vault: "v1".into(),
                proof: [0xab; 32],
            },
            Control::RetiredProof {
                vault: "v1".into(),
                proof: [0xcd; 32],
            },
            Control::ProofsDone,
            Control::Removed {
                ids: vec!["f".repeat(64)],
            },
            Control::State {
                vault: "v1".into(),
                revision: "r".into(),
                now_ms: 5,
            },
            Control::Pack {
                vault: "v1".into(),
                revision: "r".into(),
            },
        ]
    }

    #[test]
    fn a_message_names_its_kind_without_its_fields() {
        for message in one_of_each() {
            let tag = serde_json::to_value(&message).unwrap()["t"].clone();
            assert_eq!(tag, message.kind());
        }
    }

    #[test]
    fn control_messages_are_tagged_by_area_and_name() {
        let json = serde_json::to_value(&one_of_each()[1]).unwrap();
        assert_eq!(json["t"], "trust/proof");
        assert_eq!(json["proof"], "ab".repeat(32));
    }

    #[test]
    fn every_control_message_crosses_whole() {
        let (a, b) = (identity(), identity());
        let (mut dialled, mut answered) = pair(&a, &b);
        for message in one_of_each() {
            dialled.send_control(&message).unwrap();
            assert_eq!(answered.recv_control().unwrap(), message);
        }
        assert!(matches!(
            Control::hello("Laptop"),
            Control::Hello { version: PROTOCOL_VERSION, name, .. } if name == "Laptop"
        ));
    }

    #[test]
    fn an_unknown_or_malformed_control_message_is_a_protocol_error() {
        for bytes in [
            &br#"{"t":"setup/offer","vault":"v1"}"#[..],
            br#"{"t":"trust/something-new"}"#,
            br#"{"no":"tag"}"#,
            b"not json",
            br#"{"t":"trust/proof","vault":"v","proof":"zz"}"#,
        ] {
            let (a, b) = (identity(), identity());
            let (mut dialled, mut answered) = pair(&a, &b);
            dialled.send(bytes).unwrap();
            assert!(matches!(answered.recv_control(), Err(Error::Protocol(_))));
        }
    }
}
