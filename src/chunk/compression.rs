//! LZ4 compression for chunks
//!
//! Uses LZ4 for fast compression/decompression.
//! Only compresses if the result is actually smaller.

use crate::error::{Error, Result};

/// Compress data using LZ4
///
/// Returns None if compression doesn't reduce size
pub fn compress(data: &[u8], threshold: usize) -> Option<Vec<u8>> {
    if data.len() < threshold {
        return None; // Too small to bother
    }

    let compressed = lz4_flex::compress_prepend_size(data);

    // Only use compression if it actually helps
    if compressed.len() < data.len() {
        Some(compressed)
    } else {
        None
    }
}

/// Decompress LZ4 data
pub fn decompress(data: &[u8]) -> Result<Vec<u8>> {
    lz4_flex::decompress_size_prepended(data)
        .map_err(|e| Error::Decryption(format!("Decompression failed: {}", e)))
}

/// Compress data, returning original if compression doesn't help
pub fn compress_or_original(data: &[u8], threshold: usize) -> (Vec<u8>, bool) {
    match compress(data, threshold) {
        Some(compressed) => (compressed, true),
        None => (data.to_vec(), false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compress_decompress() {
        let data = b"Hello, World! Hello, World! Hello, World!";

        if let Some(compressed) = compress(data, 10) {
            let decompressed = decompress(&compressed).unwrap();
            assert_eq!(decompressed, data);
        }
    }

    #[test]
    fn test_compress_threshold() {
        let small_data = b"Hi";
        assert!(compress(small_data, 10).is_none());
    }

    #[test]
    fn test_incompressible_data() {
        // Random-like data doesn't compress well
        let data: Vec<u8> = (0..1000).map(|i| (i * 17 + 31) as u8).collect();
        let result = compress(&data, 10);

        // May or may not compress, but if it does, decompression should work
        if let Some(compressed) = result {
            let decompressed = decompress(&compressed).unwrap();
            assert_eq!(decompressed, data);
        }
    }

    #[test]
    fn test_compress_or_original() {
        let compressible = vec![0x42u8; 1000]; // Very compressible
        let (result, compressed) = compress_or_original(&compressible, 10);

        assert!(compressed);
        assert!(result.len() < compressible.len());

        let decompressed = decompress(&result).unwrap();
        assert_eq!(decompressed, compressible);
    }

    #[test]
    fn test_large_data() {
        let data = vec![0x42u8; 1024 * 1024]; // 1MB

        if let Some(compressed) = compress(&data, 1024) {
            let decompressed = decompress(&compressed).unwrap();
            assert_eq!(decompressed, data);
        }
    }
}
