use serde::{Deserialize, Serialize};

/// Serializable form of a cached upstream response. Both streaming and
/// buffered responses store as a list of chunks plus the captured status
/// + headers, so replay is uniform.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub chunks: Vec<CachedChunk>,
    pub finished_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedChunk {
    #[serde(with = "base64_bytes")]
    pub data: Vec<u8>,
}

mod base64_bytes {
    use base64::engine::general_purpose::STANDARD_NO_PAD;
    use base64::Engine;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(b: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&STANDARD_NO_PAD.encode(b))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let s: String = String::deserialize(d)?;
        STANDARD_NO_PAD
            .decode(s.as_bytes())
            .map_err(serde::de::Error::custom)
    }
}
