use std::io;

use serde::{Deserialize, Serialize};

const ZSTD_LEVEL: i32 = 3;

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
    pub data: Vec<u8>,
}

impl CachedResponse {
    pub fn encode(&self) -> io::Result<Vec<u8>> {
        let bytes =
            bincode::serde::encode_to_vec(self, bincode::config::standard()).map_err(io_invalid)?;
        zstd::encode_all(bytes.as_slice(), ZSTD_LEVEL)
    }

    pub fn decode(blob: &[u8]) -> io::Result<Self> {
        let bytes = zstd::decode_all(blob)?;
        let (value, _) = bincode::serde::decode_from_slice(&bytes, bincode::config::standard())
            .map_err(io_invalid)?;
        Ok(value)
    }
}

fn io_invalid<E: std::fmt::Display>(e: E) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> CachedResponse {
        CachedResponse {
            status: 200,
            headers: vec![
                ("content-type".into(), "application/json".into()),
                ("x-request-id".into(), "abc123".into()),
            ],
            chunks: vec![
                CachedChunk {
                    data: br#"{"id":"chatcmpl-1","object":"chat.completion","choices":[{"message":{"role":"assistant","content":"Hello, world!"}}]}"#.to_vec(),
                },
                CachedChunk {
                    data: br#"{"usage":{"prompt_tokens":10,"completion_tokens":3,"total_tokens":13}}"#.to_vec(),
                },
            ],
            finished_at_ms: 1_700_000_000_000,
        }
    }

    #[test]
    fn round_trip() {
        let original = sample();
        let blob = original.encode().expect("encode");
        let decoded = CachedResponse::decode(&blob).expect("decode");
        assert_eq!(decoded.status, original.status);
        assert_eq!(decoded.headers, original.headers);
        assert_eq!(decoded.chunks.len(), original.chunks.len());
        for (a, b) in decoded.chunks.iter().zip(original.chunks.iter()) {
            assert_eq!(a.data, b.data);
        }
        assert_eq!(decoded.finished_at_ms, original.finished_at_ms);
    }

    #[test]
    fn compresses_realistic_payload() {
        let big_text = "The quick brown fox jumps over the lazy dog. ".repeat(200);
        let payload = CachedResponse {
            status: 200,
            headers: vec![("content-type".into(), "text/event-stream".into())],
            chunks: vec![CachedChunk {
                data: big_text.into_bytes(),
            }],
            finished_at_ms: 0,
        };
        let raw = bincode::serde::encode_to_vec(&payload, bincode::config::standard()).unwrap();
        let compressed = payload.encode().unwrap();
        assert!(
            compressed.len() * 2 < raw.len(),
            "expected >=2x compression, raw={} compressed={}",
            raw.len(),
            compressed.len()
        );
    }

    #[test]
    fn decode_rejects_garbage() {
        assert!(CachedResponse::decode(b"not a zstd frame").is_err());
    }
}
