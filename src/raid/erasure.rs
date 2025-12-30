//! Reed-Solomon erasure coding for data redundancy
//!
//! Provides encode/decode operations for K-of-N erasure coding.
//! Any K shards can reconstruct the original data.

use reed_solomon_erasure::galois_8::ReedSolomon;

use crate::error::{Error, Result};

/// Reed-Solomon encoder/decoder
pub struct Encoder {
    rs: ReedSolomon,
    data_shards: usize,   // K
    parity_shards: usize, // N-K
}

impl Encoder {
    /// Create new encoder with K data shards and N total shards
    ///
    /// # Arguments
    /// * `data_shards` - K, the number of data shards
    /// * `total_shards` - N, the total number of shards (data + parity)
    ///
    /// # Errors
    /// Returns error if parameters are invalid (K must be > 0, N must be > K)
    pub fn new(data_shards: usize, total_shards: usize) -> Result<Self> {
        if data_shards == 0 {
            return Err(Error::Internal(
                "data_shards must be greater than 0".to_string(),
            ));
        }
        if total_shards <= data_shards {
            return Err(Error::Internal(
                "total_shards must be greater than data_shards".to_string(),
            ));
        }

        let parity_shards = total_shards - data_shards;

        let rs = ReedSolomon::new(data_shards, parity_shards).map_err(|e| {
            Error::Internal(format!("Failed to create Reed-Solomon encoder: {}", e))
        })?;

        Ok(Self {
            rs,
            data_shards,
            parity_shards,
        })
    }

    /// Encode data into N shards (K data + parity)
    ///
    /// The original data length is stored as the first 8 bytes (u64 big-endian)
    /// before encoding. Data is padded to be divisible by K.
    ///
    /// # Arguments
    /// * `data` - Raw data bytes to encode
    ///
    /// # Returns
    /// Vec of N shards, each shard is (data.len() + 8) / K bytes (rounded up)
    pub fn encode(&self, data: &[u8]) -> Result<Vec<Vec<u8>>> {
        // Prepend original length as u64 big-endian
        let original_len = data.len() as u64;
        let mut data_with_len = Vec::with_capacity(8 + data.len());
        data_with_len.extend_from_slice(&original_len.to_be_bytes());
        data_with_len.extend_from_slice(data);

        // Calculate shard size (pad to be divisible by data_shards)
        let shard_size = self.shard_size(data.len());

        // Pad data to be divisible by data_shards
        let total_data_len = shard_size * self.data_shards;
        data_with_len.resize(total_data_len, 0);

        // Split into data shards
        let mut shards: Vec<Vec<u8>> = data_with_len
            .chunks(shard_size)
            .map(|chunk| chunk.to_vec())
            .collect();

        // Add empty parity shards
        for _ in 0..self.parity_shards {
            shards.push(vec![0u8; shard_size]);
        }

        // Encode parity
        self.rs.encode(&mut shards).map_err(|e| {
            Error::Internal(format!("Reed-Solomon encoding failed: {}", e))
        })?;

        Ok(shards)
    }

    /// Decode from available shards back to original data
    ///
    /// # Arguments
    /// * `shards` - Vec of Option<Vec<u8>> - None for missing shards
    ///
    /// # Returns
    /// Reconstructed original data (trimmed to original length)
    ///
    /// # Errors
    /// Returns error if not enough shards are available (need at least K)
    pub fn decode(&self, shards: &mut [Option<Vec<u8>>]) -> Result<Vec<u8>> {
        let total_shards = self.data_shards + self.parity_shards;

        if shards.len() != total_shards {
            return Err(Error::Internal(format!(
                "Expected {} shards, got {}",
                total_shards,
                shards.len()
            )));
        }

        if !self.can_reconstruct(shards) {
            return Err(Error::Internal(format!(
                "Not enough shards to reconstruct: need {}, have {}",
                self.data_shards,
                shards.iter().filter(|s| s.is_some()).count()
            )));
        }

        // Reconstruct missing shards
        self.rs.reconstruct(shards).map_err(|e| {
            Error::Internal(format!("Reed-Solomon reconstruction failed: {}", e))
        })?;

        // Combine data shards
        let mut reconstructed = Vec::new();
        for shard in shards.iter().take(self.data_shards) {
            if let Some(data) = shard {
                reconstructed.extend_from_slice(data);
            } else {
                return Err(Error::Internal(
                    "Reconstruction succeeded but data shard is missing".to_string(),
                ));
            }
        }

        // Extract original length from first 8 bytes
        if reconstructed.len() < 8 {
            return Err(Error::Internal(
                "Reconstructed data too short to contain length header".to_string(),
            ));
        }

        let original_len = u64::from_be_bytes(
            reconstructed[..8]
                .try_into()
                .map_err(|_| Error::Internal("Failed to read length header".to_string()))?,
        ) as usize;

        // Validate and trim to original length
        if original_len > reconstructed.len() - 8 {
            return Err(Error::Internal(format!(
                "Invalid original length {} exceeds available data {}",
                original_len,
                reconstructed.len() - 8
            )));
        }

        Ok(reconstructed[8..8 + original_len].to_vec())
    }

    /// Get the required shard size for given data length
    ///
    /// Each shard will be ceil((data_len + 8) / data_shards) bytes,
    /// where 8 bytes are for the length header.
    pub fn shard_size(&self, data_len: usize) -> usize {
        let total_len = data_len + 8; // Include length header
        (total_len + self.data_shards - 1) / self.data_shards
    }

    /// Check if we have enough shards to reconstruct
    ///
    /// Need at least K (data_shards) shards to reconstruct.
    pub fn can_reconstruct(&self, shards: &[Option<Vec<u8>>]) -> bool {
        let available = shards.iter().filter(|s| s.is_some()).count();
        available >= self.data_shards
    }

    /// Get K (data shards)
    pub fn data_shards(&self) -> usize {
        self.data_shards
    }

    /// Get N (total shards)
    pub fn total_shards(&self) -> usize {
        self.data_shards + self.parity_shards
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encoder_creation() {
        // Valid configurations
        assert!(Encoder::new(2, 3).is_ok());
        assert!(Encoder::new(3, 5).is_ok());
        assert!(Encoder::new(4, 6).is_ok());

        // Invalid configurations
        assert!(Encoder::new(0, 3).is_err()); // K = 0
        assert!(Encoder::new(3, 3).is_err()); // N = K
        assert!(Encoder::new(5, 3).is_err()); // N < K
    }

    #[test]
    fn test_encode_decode_roundtrip_2_3() {
        let encoder = Encoder::new(2, 3).unwrap();
        let data = b"Hello, World! This is a test of erasure coding.";

        let shards = encoder.encode(data).unwrap();
        assert_eq!(shards.len(), 3);

        // All shards present
        let mut shards_opt: Vec<Option<Vec<u8>>> =
            shards.iter().map(|s| Some(s.clone())).collect();
        let decoded = encoder.decode(&mut shards_opt).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn test_encode_decode_roundtrip_3_5() {
        let encoder = Encoder::new(3, 5).unwrap();
        let data = b"Testing with 3-of-5 erasure coding configuration.";

        let shards = encoder.encode(data).unwrap();
        assert_eq!(shards.len(), 5);

        // All shards present
        let mut shards_opt: Vec<Option<Vec<u8>>> =
            shards.iter().map(|s| Some(s.clone())).collect();
        let decoded = encoder.decode(&mut shards_opt).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn test_encode_decode_roundtrip_4_6() {
        let encoder = Encoder::new(4, 6).unwrap();
        let data = b"Testing with 4-of-6 erasure coding for RAID-like redundancy.";

        let shards = encoder.encode(data).unwrap();
        assert_eq!(shards.len(), 6);

        // All shards present
        let mut shards_opt: Vec<Option<Vec<u8>>> =
            shards.iter().map(|s| Some(s.clone())).collect();
        let decoded = encoder.decode(&mut shards_opt).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn test_reconstruction_with_missing_shards_2_3() {
        let encoder = Encoder::new(2, 3).unwrap();
        let data = b"Test data for reconstruction";

        let shards = encoder.encode(data).unwrap();

        // Missing one shard (still have K=2 shards)
        let mut shards_opt: Vec<Option<Vec<u8>>> = vec![
            Some(shards[0].clone()),
            Some(shards[1].clone()),
            None, // Missing parity shard
        ];
        let decoded = encoder.decode(&mut shards_opt).unwrap();
        assert_eq!(decoded, data);

        // Missing first data shard
        let mut shards_opt: Vec<Option<Vec<u8>>> = vec![
            None, // Missing data shard
            Some(shards[1].clone()),
            Some(shards[2].clone()),
        ];
        let decoded = encoder.decode(&mut shards_opt).unwrap();
        assert_eq!(decoded, data);

        // Missing second data shard
        let mut shards_opt: Vec<Option<Vec<u8>>> = vec![
            Some(shards[0].clone()),
            None, // Missing data shard
            Some(shards[2].clone()),
        ];
        let decoded = encoder.decode(&mut shards_opt).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn test_reconstruction_with_missing_shards_3_5() {
        let encoder = Encoder::new(3, 5).unwrap();
        let data = b"Test data for 3-of-5 reconstruction";

        let shards = encoder.encode(data).unwrap();

        // Missing two shards (still have K=3 shards)
        let mut shards_opt: Vec<Option<Vec<u8>>> = vec![
            Some(shards[0].clone()),
            None,
            Some(shards[2].clone()),
            None,
            Some(shards[4].clone()),
        ];
        let decoded = encoder.decode(&mut shards_opt).unwrap();
        assert_eq!(decoded, data);

        // Missing first two data shards
        let mut shards_opt: Vec<Option<Vec<u8>>> = vec![
            None,
            None,
            Some(shards[2].clone()),
            Some(shards[3].clone()),
            Some(shards[4].clone()),
        ];
        let decoded = encoder.decode(&mut shards_opt).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn test_reconstruction_with_missing_shards_4_6() {
        let encoder = Encoder::new(4, 6).unwrap();
        let data = b"Test data for 4-of-6 reconstruction scenario";

        let shards = encoder.encode(data).unwrap();

        // Missing two shards (still have K=4 shards)
        let mut shards_opt: Vec<Option<Vec<u8>>> = vec![
            Some(shards[0].clone()),
            None,
            Some(shards[2].clone()),
            Some(shards[3].clone()),
            None,
            Some(shards[5].clone()),
        ];
        let decoded = encoder.decode(&mut shards_opt).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn test_not_enough_shards() {
        let encoder = Encoder::new(3, 5).unwrap();
        let data = b"Test data";

        let shards = encoder.encode(data).unwrap();

        // Only 2 shards (need 3)
        let mut shards_opt: Vec<Option<Vec<u8>>> = vec![
            Some(shards[0].clone()),
            Some(shards[1].clone()),
            None,
            None,
            None,
        ];

        assert!(encoder.decode(&mut shards_opt).is_err());
    }

    #[test]
    fn test_can_reconstruct() {
        let encoder = Encoder::new(3, 5).unwrap();
        let data = b"Test";

        let shards = encoder.encode(data).unwrap();

        // Enough shards (3)
        let shards_opt: Vec<Option<Vec<u8>>> = vec![
            Some(shards[0].clone()),
            Some(shards[1].clone()),
            Some(shards[2].clone()),
            None,
            None,
        ];
        assert!(encoder.can_reconstruct(&shards_opt));

        // Not enough shards (2)
        let shards_opt: Vec<Option<Vec<u8>>> = vec![
            Some(shards[0].clone()),
            Some(shards[1].clone()),
            None,
            None,
            None,
        ];
        assert!(!encoder.can_reconstruct(&shards_opt));

        // Exactly K shards
        let shards_opt: Vec<Option<Vec<u8>>> = vec![
            None,
            Some(shards[1].clone()),
            Some(shards[2].clone()),
            Some(shards[3].clone()),
            None,
        ];
        assert!(encoder.can_reconstruct(&shards_opt));
    }

    #[test]
    fn test_shard_size() {
        let encoder = Encoder::new(3, 5).unwrap();

        // 100 bytes + 8 byte header = 108 bytes, ceil(108/3) = 36
        assert_eq!(encoder.shard_size(100), 36);

        // 9 bytes + 8 byte header = 17 bytes, ceil(17/3) = 6
        assert_eq!(encoder.shard_size(9), 6);

        // 0 bytes + 8 byte header = 8 bytes, ceil(8/3) = 3
        assert_eq!(encoder.shard_size(0), 3);
    }

    #[test]
    fn test_accessors() {
        let encoder = Encoder::new(3, 5).unwrap();
        assert_eq!(encoder.data_shards(), 3);
        assert_eq!(encoder.total_shards(), 5);
    }

    #[test]
    fn test_empty_data() {
        let encoder = Encoder::new(2, 3).unwrap();
        let data: &[u8] = b"";

        let shards = encoder.encode(data).unwrap();
        assert_eq!(shards.len(), 3);

        let mut shards_opt: Vec<Option<Vec<u8>>> =
            shards.iter().map(|s| Some(s.clone())).collect();
        let decoded = encoder.decode(&mut shards_opt).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn test_large_data() {
        let encoder = Encoder::new(4, 6).unwrap();
        // 1MB of data
        let data: Vec<u8> = (0..1_000_000).map(|i| (i % 256) as u8).collect();

        let shards = encoder.encode(&data).unwrap();
        assert_eq!(shards.len(), 6);

        // Verify shard sizes are consistent
        let shard_size = shards[0].len();
        for shard in &shards {
            assert_eq!(shard.len(), shard_size);
        }

        // Reconstruct with missing shards
        let mut shards_opt: Vec<Option<Vec<u8>>> = vec![
            Some(shards[0].clone()),
            None,
            Some(shards[2].clone()),
            Some(shards[3].clone()),
            Some(shards[4].clone()),
            None,
        ];
        let decoded = encoder.decode(&mut shards_opt).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn test_data_not_divisible_by_k() {
        let encoder = Encoder::new(3, 5).unwrap();
        // 17 bytes - not divisible by 3
        let data = b"12345678901234567";

        let shards = encoder.encode(data).unwrap();
        assert_eq!(shards.len(), 5);

        let mut shards_opt: Vec<Option<Vec<u8>>> =
            shards.iter().map(|s| Some(s.clone())).collect();
        let decoded = encoder.decode(&mut shards_opt).unwrap();
        assert_eq!(decoded, data.as_slice());
    }

    #[test]
    fn test_all_missing_combinations_2_3() {
        let encoder = Encoder::new(2, 3).unwrap();
        let data = b"All combinations test";

        let shards = encoder.encode(data).unwrap();

        // Test all valid combinations (any 2 of 3)
        let valid_combinations = [
            [true, true, false],
            [true, false, true],
            [false, true, true],
            [true, true, true],
        ];

        for combo in valid_combinations {
            let mut shards_opt: Vec<Option<Vec<u8>>> = shards
                .iter()
                .enumerate()
                .map(|(i, s)| if combo[i] { Some(s.clone()) } else { None })
                .collect();
            let decoded = encoder.decode(&mut shards_opt).unwrap();
            assert_eq!(decoded, data.as_slice(), "Failed for combination {:?}", combo);
        }
    }
}
