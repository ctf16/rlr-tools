// Merkle Tree for Rocket League replay verification
// 1. Split replay JSON into semantic sections -> hash each section
// 2. Concatenate hashed sections -> hash the result to get parent
// 3. Repeat until root
// 4. Sign the root with hybrid Ed25519 + ML-DSA-65

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use ed25519_dalek::{Signer as Ed25519Signer, SigningKey, Verifier as Ed25519Verifier, VerifyingKey};
use fips204::ml_dsa_65;
use fips204::traits::{SerDes, Signer as MlDsaSigner, Verifier as MlDsaVerifier};
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha3::{Digest, Sha3_256};
use std::error;
use std::fs;

pub type Hash = [u8; 32];

pub const SECTION_LABELS: &[&str] = &[
    "Header",
    "Match Metadata",
    "Goals",
    "Player Stats",
    "Network Frames",
    "Content & Indices",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MerkleTree {
    pub leaves: Vec<Hash>,
    pub nodes: Vec<Hash>,
    pub root: Hash,
}

#[derive(Debug)]
pub enum VerifyResult {
    Valid,
    Tampered { section_index: Option<usize> },
}

#[derive(Serialize, Deserialize)]
pub struct SidecarFile {
    pub algorithm: String,
    pub ed25519_public_key: Vec<u8>,
    pub ed25519_signature: Vec<u8>,
    pub mldsa65_public_key: String,
    pub mldsa65_signature: String,
    pub merkle: MerkleTree,
}

fn hash_section(data: &[u8]) -> Hash {
    Sha3_256::digest(data).into()
}

fn hash_pair(left: &Hash, right: &Hash) -> Hash {
    let mut hasher = Sha3_256::new();
    hasher.update(left);
    hasher.update(right);
    hasher.finalize().into()
}

fn build_tree(leaves: &[Hash]) -> Vec<Hash> {
    let n = leaves.len().next_power_of_two();
    let mut tree = vec![[0u8; 32]; 2 * n - 1];

    let offset = n - 1;
    for (i, leaf) in leaves.iter().enumerate() {
        tree[offset + i] = *leaf;
    }

    // Duplicate last leaf to fill power-of-two
    for i in leaves.len()..n {
        tree[offset + i] = tree[offset + leaves.len() - 1];
    }

    for i in (0..offset).rev() {
        let left = tree[2 * i + 1];
        let right = tree[2 * i + 2];
        tree[i] = hash_pair(&left, &right);
    }

    tree
}

/// Split a parsed replay JSON into semantic sections for Merkle hashing.
///
/// Sections:
///   0. Header — version info, game_type, CRCs
///   1. Match Metadata — properties (excluding Goals and PlayerStats)
///   2. Goals — chronological goal list
///   3. Player Stats — per-player scoreboard
///   4. Network Frames — tick-by-tick physics/input data
///   5. Content & Indices — levels, keyframes, tick_marks, packages, objects, etc.
pub fn split_replay_json(json: &Value) -> Vec<Vec<u8>> {
    let mut sections = Vec::new();

    // Section 0: Header
    let header = json!({
        "header_size": json["header_size"],
        "header_crc": json["header_crc"],
        "major_version": json["major_version"],
        "minor_version": json["minor_version"],
        "net_version": json["net_version"],
        "game_type": json["game_type"],
    });
    sections.push(serde_json::to_vec(&header).unwrap());

    // Section 1: Match metadata (properties minus Goals and PlayerStats)
    let mut match_info = serde_json::Map::new();
    if let Some(obj) = json["properties"].as_object() {
        for (k, v) in obj {
            if k != "Goals" && k != "PlayerStats" {
                match_info.insert(k.clone(), v.clone());
            }
        }
    }
    sections.push(serde_json::to_vec(&Value::Object(match_info)).unwrap());

    // Section 2: Goals
    sections.push(serde_json::to_vec(&json["properties"]["Goals"]).unwrap());

    // Section 3: Player stats
    sections.push(serde_json::to_vec(&json["properties"]["PlayerStats"]).unwrap());

    // Section 4: Network frames
    sections.push(serde_json::to_vec(&json["network_frames"]).unwrap());

    // Section 5: Remaining content
    let remaining = json!({
        "content_size": json["content_size"],
        "content_crc": json["content_crc"],
        "levels": json["levels"],
        "keyframes": json["keyframes"],
        "debug_info": json["debug_info"],
        "tick_marks": json["tick_marks"],
        "packages": json["packages"],
        "objects": json["objects"],
        "names": json["names"],
        "class_indices": json["class_indices"],
        "net_cache": json["net_cache"],
    });
    sections.push(serde_json::to_vec(&remaining).unwrap());

    sections
}

impl MerkleTree {
    pub fn new(sections: &[&[u8]]) -> Self {
        assert!(!sections.is_empty(), "Need at least one section");

        let leaves: Vec<Hash> = sections.iter().map(|s| hash_section(s)).collect();
        let nodes = build_tree(&leaves);
        let root = nodes[0];

        MerkleTree { leaves, nodes, root }
    }

    pub fn from_replay_json(json: &Value) -> Self {
        let sections = split_replay_json(json);
        let refs: Vec<&[u8]> = sections.iter().map(|s| s.as_slice()).collect();
        Self::new(&refs)
    }

    pub fn verify(&self, sections: &[&[u8]]) -> VerifyResult {
        if sections.len() != self.leaves.len() {
            return VerifyResult::Tampered { section_index: None };
        }

        for (i, section) in sections.iter().enumerate() {
            let hash = hash_section(section);
            if hash != self.leaves[i] {
                return VerifyResult::Tampered {
                    section_index: Some(i),
                };
            }
        }

        VerifyResult::Valid
    }

    pub fn verify_replay_json(&self, json: &Value) -> VerifyResult {
        let sections = split_replay_json(json);
        let refs: Vec<&[u8]> = sections.iter().map(|s| s.as_slice()).collect();
        self.verify(&refs)
    }
}

impl SidecarFile {
    pub fn create(tree: MerkleTree) -> Result<Self, Box<dyn error::Error>> {
        // Ed25519 signing
        let ed_signing_key = SigningKey::generate(&mut OsRng);
        let ed_signature = ed_signing_key.sign(&tree.root);
        let ed_verifying_key = ed_signing_key.verifying_key();

        // ML-DSA-65 signing
        let (mldsa_pk, mldsa_sk) = ml_dsa_65::try_keygen()?;
        let mldsa_signature = mldsa_sk.try_sign(&tree.root, &[])?;

        Ok(SidecarFile {
            algorithm: "hybrid-ed25519-mldsa65".to_string(),
            ed25519_public_key: ed_verifying_key.as_bytes().to_vec(),
            ed25519_signature: ed_signature.to_bytes().to_vec(),
            mldsa65_public_key: BASE64.encode(mldsa_pk.into_bytes()),
            mldsa65_signature: BASE64.encode(mldsa_signature),
            merkle: tree,
        })
    }

    pub fn verify_signature(&self) -> HybridVerifyResult {
        let ed25519_ok = self.verify_ed25519();
        let mldsa65_ok = self.verify_mldsa65();

        HybridVerifyResult {
            ed25519_ok,
            mldsa65_ok,
        }
    }

    fn verify_ed25519(&self) -> bool {
        let Ok(pub_bytes): Result<[u8; 32], _> = self.ed25519_public_key.as_slice().try_into()
        else {
            return false;
        };
        let Ok(verifying_key) = VerifyingKey::from_bytes(&pub_bytes) else {
            return false;
        };
        let Ok(sig_bytes): Result<[u8; 64], _> = self.ed25519_signature.as_slice().try_into()
        else {
            return false;
        };
        let signature = ed25519_dalek::Signature::from_bytes(&sig_bytes);
        verifying_key.verify(&self.merkle.root, &signature).is_ok()
    }

    fn verify_mldsa65(&self) -> bool {
        let Ok(pk_bytes) = BASE64.decode(&self.mldsa65_public_key) else {
            return false;
        };
        let Ok(pk_array): Result<[u8; ml_dsa_65::PK_LEN], _> = pk_bytes.as_slice().try_into()
        else {
            return false;
        };
        let Ok(pk) = ml_dsa_65::PublicKey::try_from_bytes(pk_array) else {
            return false;
        };
        let Ok(sig_bytes) = BASE64.decode(&self.mldsa65_signature) else {
            return false;
        };
        let Ok(sig_array): Result<[u8; ml_dsa_65::SIG_LEN], _> = sig_bytes.as_slice().try_into()
        else {
            return false;
        };
        pk.verify(&self.merkle.root, &sig_array, &[])
    }

    pub fn save(&self, path: &str) -> Result<(), Box<dyn error::Error>> {
        let json = serde_json::to_string_pretty(self)?;
        fs::write(path, json)?;
        Ok(())
    }

    pub fn load(path: &str) -> Result<Self, Box<dyn error::Error>> {
        let content = fs::read_to_string(path)?;
        let sidecar: SidecarFile = serde_json::from_str(&content)?;
        Ok(sidecar)
    }
}

pub struct HybridVerifyResult {
    pub ed25519_ok: bool,
    pub mldsa65_ok: bool,
}

impl HybridVerifyResult {
    pub fn both_valid(&self) -> bool {
        self.ed25519_ok && self.mldsa65_ok
    }

    pub fn to_json(&self) -> Value {
        json!({
            "ed25519": self.ed25519_ok,
            "mldsa65": self.mldsa65_ok,
            "hybrid_valid": self.both_valid(),
        })
    }
}

impl VerifyResult {
    pub fn to_json(&self) -> Value {
        match self {
            VerifyResult::Valid => json!({
                "integrity": "valid",
                "tampered_section": null,
            }),
            VerifyResult::Tampered { section_index } => {
                let label = section_index
                    .and_then(|i| SECTION_LABELS.get(i).copied());
                json!({
                    "integrity": "tampered",
                    "tampered_section": section_index,
                    "tampered_section_label": label,
                })
            }
        }
    }
}
