use std::sync::OnceLock;

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

type HmacSha256 = Hmac<Sha256>;

const DIFFICULTY: u8 = 16;
const MAX_AGE_SECS: i64 = 300;

static CONFIG: OnceLock<PowConfig> = OnceLock::new();

struct PowConfig {
    secret: [u8; 32],
}

#[derive(serde::Serialize)]
pub struct Challenge {
    pub challenge: String,
    pub timestamp: i64,
    pub signature: String,
    pub difficulty: u8,
}

#[derive(serde::Deserialize)]
pub struct PowSolution {
    pub challenge: String,
    pub timestamp: i64,
    pub signature: String,
    pub nonce: u64,
}

fn config() -> &'static PowConfig {
    CONFIG.get_or_init(|| {
        use rand::Rng;
        PowConfig {
            secret: rand::rng().random(),
        }
    })
}

pub fn generate() -> Challenge {
    use rand::Rng;
    let bytes: [u8; 16] = rand::rng().random();
    let challenge: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    let timestamp = chrono::Utc::now().timestamp();
    let signature = config().sign(&challenge, timestamp);
    Challenge {
        challenge,
        timestamp,
        signature,
        difficulty: DIFFICULTY,
    }
}

pub fn verify(sol: &PowSolution) -> bool {
    let now = chrono::Utc::now().timestamp();
    if now - sol.timestamp > MAX_AGE_SECS || sol.timestamp > now + 60 {
        return false;
    }
    if sol.signature != config().sign(&sol.challenge, sol.timestamp) {
        return false;
    }
    let mut hasher = Sha256::new();
    hasher.update(sol.challenge.as_bytes());
    hasher.update(&sol.nonce.to_le_bytes());
    let hash = hasher.finalize();
    has_leading_zeros(&hash, DIFFICULTY)
}

impl PowConfig {
    fn sign(&self, challenge: &str, timestamp: i64) -> String {
        let mut mac = HmacSha256::new_from_slice(&self.secret).unwrap();
        mac.update(challenge.as_bytes());
        mac.update(&timestamp.to_le_bytes());
        mac.finalize()
            .into_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }
}

fn has_leading_zeros(hash: &[u8], bits: u8) -> bool {
    let full = (bits / 8) as usize;
    let rem = bits % 8;
    for &b in &hash[..full] {
        if b != 0 {
            return false;
        }
    }
    rem == 0 || hash[full] >> (8 - rem) == 0
}
