//! Stripe management for erasure-coded chunks
//!
//! Handles splitting data into stripes, assigning blocks to accounts,
//! and reconstructing data from available blocks.

use crate::chunk::{BlockLocation, ChunkId, StripeInfo};
use crate::error::{Error, Result};

use super::erasure::Encoder;

/// A stripe ready for upload or after download
pub struct Stripe {
    /// Chunk ID this stripe belongs to
    pub chunk_id: ChunkId,
    /// Block data (index corresponds to block_index)
    pub blocks: Vec<Vec<u8>>,
    /// Account assignments for each block
    pub assignments: Vec<u8>,
    /// Number of data blocks (K)
    pub data_count: usize,
}

impl Stripe {
    /// Get the stripe/chunk ID
    ///
    /// Returns the chunk_id for compatibility with pool operations
    pub fn stripe_id(&self) -> &str {
        &self.chunk_id
    }

    /// Get the number of data blocks (K)
    pub fn data_count(&self) -> u8 {
        self.data_count as u8
    }

    /// Get the block assigned to a specific account
    ///
    /// Returns `Some((block_index, data))` if this account has a block assignment,
    /// `None` otherwise.
    pub fn block_for_account(&self, account_id: u8) -> Option<(u8, &[u8])> {
        for (block_index, &assigned_account) in self.assignments.iter().enumerate() {
            if assigned_account == account_id {
                return Some((block_index as u8, &self.blocks[block_index]));
            }
        }
        None
    }

    /// Get all (block_index, account_id, data) tuples
    pub fn all_blocks(&self) -> Vec<(u8, u8, &[u8])> {
        self.blocks
            .iter()
            .enumerate()
            .zip(self.assignments.iter())
            .map(|((block_idx, data), &account_id)| (block_idx as u8, account_id, data.as_slice()))
            .collect()
    }

    /// Get the total number of blocks (N)
    pub fn total_blocks(&self) -> usize {
        self.blocks.len()
    }

    /// Get the number of parity blocks (N - K)
    pub fn parity_count(&self) -> u8 {
        (self.blocks.len() - self.data_count) as u8
    }

    /// Get the block size (all blocks have the same size)
    pub fn block_size(&self) -> usize {
        self.blocks.first().map(|b| b.len()).unwrap_or(0)
    }
}

/// Manages stripe creation and reconstruction
pub struct StripeManager {
    encoder: Encoder,
    num_accounts: usize,
}

impl StripeManager {
    /// Create a new stripe manager
    ///
    /// # Arguments
    /// * `data_shards` - Number of data shards (K)
    /// * `total_shards` - Total number of shards including parity (N)
    /// * `num_accounts` - Number of accounts in the pool
    ///
    /// # Errors
    /// Returns error if the encoder configuration is invalid
    pub fn new(data_shards: usize, total_shards: usize, num_accounts: usize) -> Result<Self> {
        if num_accounts == 0 {
            return Err(Error::Config("num_accounts must be > 0".to_string()));
        }
        if num_accounts < total_shards {
            return Err(Error::Config(format!(
                "num_accounts ({}) must be >= total_shards ({})",
                num_accounts, total_shards
            )));
        }

        let encoder = Encoder::new(data_shards, total_shards)?;

        Ok(StripeManager {
            encoder,
            num_accounts,
        })
    }

    /// Create a stripe from chunk data
    ///
    /// Encodes data using Reed-Solomon and assigns blocks to accounts
    /// using rotating parity distribution.
    ///
    /// # Arguments
    /// * `chunk_id` - The ID of the chunk this stripe belongs to
    /// * `data` - The raw data to encode
    /// * `stripe_index` - Index used to rotate parity assignments
    ///
    /// # Returns
    /// A `Stripe` containing encoded blocks and account assignments
    pub fn create_stripe(&self, chunk_id: ChunkId, data: &[u8], stripe_index: u64) -> Result<Stripe> {
        // Encode data into shards
        let blocks = self.encoder.encode(data)?;

        // Get account assignments for this stripe
        let assignments = self.get_assignments(stripe_index);

        Ok(Stripe {
            chunk_id,
            blocks,
            assignments,
            data_count: self.encoder.data_shards(),
        })
    }

    /// Get account assignment for each block in a stripe
    ///
    /// Uses rotating parity distribution (like RAID5/6).
    /// The stripe_index is used to rotate which account gets parity blocks.
    ///
    /// # Algorithm
    /// For N total shards and stripe_index i:
    /// - Parity blocks rotate through accounts to spread load evenly
    /// - Example with 4 accounts, 3 data + 1 parity:
    ///   - Stripe 0: [D0->A0, D1->A1, D2->A2, P->A3]
    ///   - Stripe 1: [D0->A0, D1->A1, P->A2, D2->A3]
    ///   - Stripe 2: [D0->A0, P->A1, D1->A2, D2->A3]
    ///   - Stripe 3: [P->A0, D0->A1, D1->A2, D2->A3]
    ///
    /// # Arguments
    /// * `stripe_index` - Index used to determine parity rotation
    ///
    /// # Returns
    /// Vector of account IDs, where index corresponds to block_index
    pub fn get_assignments(&self, stripe_index: u64) -> Vec<u8> {
        let total_shards = self.encoder.total_shards();

        // Calculate rotation offset based on stripe index
        // This rotates which position(s) get parity blocks
        let rotation = (stripe_index as usize) % total_shards;

        // Build assignment array
        // For each position, determine if it should be data or parity
        // based on rotation
        let mut assignments = Vec::with_capacity(total_shards);

        // The parity blocks are at positions that "rotate" through the stripe
        // For stripe_index 0: parity at positions [K, K+1, ..., N-1]
        // For stripe_index 1: parity rotates left by 1
        // etc.
        for block_idx in 0..total_shards {
            // Calculate which account this block goes to
            // Account ID = block position in the rotated assignment
            //
            // We want to distribute blocks across accounts evenly.
            // The simple approach: account_id = block_idx % num_accounts
            // But we also want to rotate parity positions.
            //
            // For rotating parity: we shift assignments based on stripe_index
            let account_id = (block_idx + rotation) % self.num_accounts;
            assignments.push(account_id as u8);
        }

        assignments
    }

    /// Reconstruct chunk data from available blocks
    ///
    /// # Arguments
    /// * `blocks` - Vec of (block_index, data) for available blocks
    ///
    /// # Returns
    /// Reconstructed original data
    ///
    /// # Errors
    /// Returns error if not enough blocks are available (need at least K)
    pub fn reconstruct(&self, blocks: &[(u8, Vec<u8>)]) -> Result<Vec<u8>> {
        let total_shards = self.encoder.total_shards();

        // Build the shard array for the encoder
        let mut shards: Vec<Option<Vec<u8>>> = vec![None; total_shards];

        for (block_index, data) in blocks {
            let idx = *block_index as usize;
            if idx >= total_shards {
                return Err(Error::Internal(format!(
                    "Block index {} out of range (max {})",
                    idx,
                    total_shards - 1
                )));
            }
            shards[idx] = Some(data.clone());
        }

        // Decode using the encoder
        self.encoder.decode(&mut shards)
    }

    /// Convert Stripe to StripeInfo (after upload with message IDs)
    ///
    /// # Arguments
    /// * `stripe` - The stripe that was uploaded
    /// * `message_ids` - Vec of (block_index, message_id) from successful uploads
    ///
    /// # Returns
    /// A `StripeInfo` structure with block locations
    pub fn to_stripe_info(&self, stripe: &Stripe, message_ids: &[(u8, i32)]) -> StripeInfo {
        let total_shards = self.encoder.total_shards();
        let data_shards = self.encoder.data_shards();
        let parity_shards = total_shards - data_shards;
        let block_size = stripe.block_size() as u64;

        // Build message ID lookup
        let message_id_map: std::collections::HashMap<u8, i32> =
            message_ids.iter().cloned().collect();

        // Create block locations
        let blocks: Vec<BlockLocation> = (0..total_shards)
            .map(|block_idx| {
                let account_id = stripe.assignments[block_idx];
                let message_id = message_id_map.get(&(block_idx as u8)).copied();
                let uploaded_at = if message_id.is_some() {
                    Some(
                        std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs() as i64)
                            .unwrap_or(0),
                    )
                } else {
                    None
                };

                BlockLocation {
                    account_id,
                    message_id,
                    block_index: block_idx as u8,
                    uploaded_at,
                }
            })
            .collect();

        StripeInfo {
            blocks,
            data_count: data_shards as u8,
            parity_count: parity_shards as u8,
            block_size,
        }
    }

    /// Get the number of data shards (K)
    pub fn data_shards(&self) -> usize {
        self.encoder.data_shards()
    }

    /// Get the total number of shards (N)
    pub fn total_shards(&self) -> usize {
        self.encoder.total_shards()
    }

    /// Get the number of parity shards
    pub fn parity_shards(&self) -> usize {
        self.encoder.total_shards() - self.encoder.data_shards()
    }

    /// Get the number of accounts
    pub fn num_accounts(&self) -> usize {
        self.num_accounts
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_stripe_manager_creation() {
        // Valid configuration
        let manager = StripeManager::new(2, 3, 3).unwrap();
        assert_eq!(manager.data_shards(), 2);
        assert_eq!(manager.total_shards(), 3);
        assert_eq!(manager.parity_shards(), 1);
        assert_eq!(manager.num_accounts(), 3);
    }

    #[test]
    fn test_stripe_manager_invalid_config() {
        // No accounts
        assert!(StripeManager::new(2, 3, 0).is_err());

        // Not enough accounts for shards
        assert!(StripeManager::new(2, 3, 2).is_err());

        // Invalid shard config (delegated to Encoder)
        assert!(StripeManager::new(0, 3, 3).is_err());
        assert!(StripeManager::new(3, 3, 3).is_err());
    }

    #[test]
    fn test_create_stripe_basic() {
        let manager = StripeManager::new(2, 3, 3).unwrap();
        let data = b"Hello, World! Test data for stripe encoding.";
        let chunk_id = "test_chunk_123".to_string();

        let stripe = manager.create_stripe(chunk_id.clone(), data, 0).unwrap();

        assert_eq!(stripe.chunk_id, chunk_id);
        assert_eq!(stripe.blocks.len(), 3);
        assert_eq!(stripe.assignments.len(), 3);
        assert_eq!(stripe.data_count, 2);
    }

    #[test]
    fn test_stripe_all_blocks() {
        let manager = StripeManager::new(2, 3, 3).unwrap();
        let data = b"Test data";
        let chunk_id = "test".to_string();

        let stripe = manager.create_stripe(chunk_id, data, 0).unwrap();
        let all_blocks = stripe.all_blocks();

        assert_eq!(all_blocks.len(), 3);
        for (block_idx, account_id, _data) in all_blocks {
            assert!(block_idx < 3);
            assert!(account_id < 3);
        }
    }

    #[test]
    fn test_stripe_block_for_account() {
        let manager = StripeManager::new(2, 3, 3).unwrap();
        let data = b"Test data";
        let chunk_id = "test".to_string();

        let stripe = manager.create_stripe(chunk_id, data, 0).unwrap();

        // Each account should have one block (with 3 accounts and 3 blocks)
        for account_id in 0..3 {
            let block = stripe.block_for_account(account_id);
            assert!(block.is_some(), "Account {} should have a block", account_id);
        }

        // Non-existent account should return None
        assert!(stripe.block_for_account(99).is_none());
    }

    #[test]
    fn test_assignment_rotation_raid5_3accounts() {
        let manager = StripeManager::new(2, 3, 3).unwrap();

        // Check that assignments rotate with stripe_index
        let assign0 = manager.get_assignments(0);
        let assign1 = manager.get_assignments(1);
        let assign2 = manager.get_assignments(2);

        // With 3 shards and 3 accounts, each rotation should be different
        assert_ne!(assign0, assign1);
        assert_ne!(assign1, assign2);

        // After 3 rotations, should cycle back
        let assign3 = manager.get_assignments(3);
        assert_eq!(assign0, assign3);

        // Verify each stripe uses all accounts
        for assignments in [&assign0, &assign1, &assign2] {
            let mut used: Vec<bool> = vec![false; 3];
            for &account in assignments {
                used[account as usize] = true;
            }
            assert!(used.iter().all(|&x| x), "All accounts should be used");
        }
    }

    #[test]
    fn test_assignment_rotation_raid5_4accounts() {
        let manager = StripeManager::new(3, 4, 4).unwrap();

        // With 4 shards and 4 accounts, parity rotates through positions
        let assign0 = manager.get_assignments(0);
        let assign1 = manager.get_assignments(1);
        let assign2 = manager.get_assignments(2);
        let assign3 = manager.get_assignments(3);

        // Each assignment should be unique within one cycle
        assert_ne!(assign0, assign1);
        assert_ne!(assign1, assign2);
        assert_ne!(assign2, assign3);

        // After 4 rotations, should cycle back
        let assign4 = manager.get_assignments(4);
        assert_eq!(assign0, assign4);
    }

    #[test]
    fn test_create_and_reconstruct_all_blocks() {
        let manager = StripeManager::new(2, 3, 3).unwrap();
        let data = b"Hello, World! This is test data for reconstruction.";
        let chunk_id = "test_chunk".to_string();

        let stripe = manager.create_stripe(chunk_id, data, 0).unwrap();

        // Reconstruct with all blocks
        let all_blocks: Vec<(u8, Vec<u8>)> = stripe
            .blocks
            .iter()
            .enumerate()
            .map(|(i, b)| (i as u8, b.clone()))
            .collect();

        let reconstructed = manager.reconstruct(&all_blocks).unwrap();
        assert_eq!(reconstructed, data);
    }

    #[test]
    fn test_reconstruct_with_missing_block() {
        let manager = StripeManager::new(2, 3, 3).unwrap();
        let data = b"Test data for reconstruction with missing block";
        let chunk_id = "test".to_string();

        let stripe = manager.create_stripe(chunk_id, data, 0).unwrap();

        // Reconstruct with only 2 blocks (minimum K=2)
        let partial_blocks: Vec<(u8, Vec<u8>)> = vec![
            (0, stripe.blocks[0].clone()),
            (1, stripe.blocks[1].clone()),
            // Missing block 2
        ];

        let reconstructed = manager.reconstruct(&partial_blocks).unwrap();
        assert_eq!(reconstructed, data);
    }

    #[test]
    fn test_reconstruct_with_parity_block() {
        let manager = StripeManager::new(2, 3, 3).unwrap();
        let data = b"Test reconstruction using parity";
        let chunk_id = "test".to_string();

        let stripe = manager.create_stripe(chunk_id, data, 0).unwrap();

        // Reconstruct using one data block and one parity block
        let partial_blocks: Vec<(u8, Vec<u8>)> = vec![
            (0, stripe.blocks[0].clone()), // Data block
            (2, stripe.blocks[2].clone()), // Parity block
        ];

        let reconstructed = manager.reconstruct(&partial_blocks).unwrap();
        assert_eq!(reconstructed, data);
    }

    #[test]
    fn test_reconstruct_insufficient_blocks() {
        let manager = StripeManager::new(3, 5, 5).unwrap();
        let data = b"Test data";
        let chunk_id = "test".to_string();

        let stripe = manager.create_stripe(chunk_id, data, 0).unwrap();

        // Only 2 blocks (need K=3)
        let partial_blocks: Vec<(u8, Vec<u8>)> = vec![
            (0, stripe.blocks[0].clone()),
            (1, stripe.blocks[1].clone()),
        ];

        assert!(manager.reconstruct(&partial_blocks).is_err());
    }

    #[test]
    fn test_to_stripe_info() {
        let manager = StripeManager::new(2, 3, 3).unwrap();
        let data = b"Test data for StripeInfo";
        let chunk_id = "test".to_string();

        let stripe = manager.create_stripe(chunk_id, data, 0).unwrap();

        // Simulate message IDs from uploads
        let message_ids: Vec<(u8, i32)> = vec![(0, 100), (1, 101), (2, 102)];

        let stripe_info = manager.to_stripe_info(&stripe, &message_ids);

        assert_eq!(stripe_info.data_count, 2);
        assert_eq!(stripe_info.parity_count, 1);
        assert_eq!(stripe_info.blocks.len(), 3);
        assert!(stripe_info.block_size > 0);

        // Verify block locations
        for block in &stripe_info.blocks {
            assert!(block.message_id.is_some());
            assert!(block.uploaded_at.is_some());
        }
    }

    #[test]
    fn test_to_stripe_info_partial_upload() {
        let manager = StripeManager::new(2, 3, 3).unwrap();
        let data = b"Test data";
        let chunk_id = "test".to_string();

        let stripe = manager.create_stripe(chunk_id, data, 0).unwrap();

        // Only 2 blocks uploaded (block 1 failed)
        let message_ids: Vec<(u8, i32)> = vec![(0, 100), (2, 102)];

        let stripe_info = manager.to_stripe_info(&stripe, &message_ids);

        // Block 0 should have message_id
        assert!(stripe_info.blocks[0].message_id.is_some());

        // Block 1 should NOT have message_id
        assert!(stripe_info.blocks[1].message_id.is_none());

        // Block 2 should have message_id
        assert!(stripe_info.blocks[2].message_id.is_some());
    }

    #[test]
    fn test_stripe_methods() {
        let manager = StripeManager::new(2, 3, 3).unwrap();
        let data = b"Test data";
        let chunk_id = "test".to_string();

        let stripe = manager.create_stripe(chunk_id, data, 0).unwrap();

        assert_eq!(stripe.total_blocks(), 3);
        assert_eq!(stripe.parity_count(), 1);
        assert!(stripe.block_size() > 0);
    }

    #[test]
    fn test_various_data_sizes() {
        let manager = StripeManager::new(3, 5, 5).unwrap();

        // Test various data sizes
        for size in [1, 10, 100, 1000, 10000] {
            let data: Vec<u8> = (0..size).map(|i| (i % 256) as u8).collect();
            let chunk_id = format!("test_{}", size);

            let stripe = manager.create_stripe(chunk_id, &data, 0).unwrap();

            // Reconstruct with all blocks
            let all_blocks: Vec<(u8, Vec<u8>)> = stripe
                .blocks
                .iter()
                .enumerate()
                .map(|(i, b)| (i as u8, b.clone()))
                .collect();

            let reconstructed = manager.reconstruct(&all_blocks).unwrap();
            assert_eq!(reconstructed, data, "Failed for size {}", size);
        }
    }

    #[test]
    fn test_all_reconstruction_combinations_2_3() {
        let manager = StripeManager::new(2, 3, 3).unwrap();
        let data = b"Test all reconstruction combinations";
        let chunk_id = "test".to_string();

        let stripe = manager.create_stripe(chunk_id, data, 0).unwrap();

        // All valid combinations of 2 blocks out of 3
        let combinations = [
            vec![0, 1],
            vec![0, 2],
            vec![1, 2],
            vec![0, 1, 2], // All blocks
        ];

        for indices in combinations {
            let blocks: Vec<(u8, Vec<u8>)> = indices
                .iter()
                .map(|&i| (i, stripe.blocks[i as usize].clone()))
                .collect();

            let reconstructed = manager.reconstruct(&blocks).unwrap();
            assert_eq!(
                reconstructed, data,
                "Failed for combination {:?}",
                indices
            );
        }
    }

    #[test]
    fn test_assignment_spread_across_stripes() {
        let manager = StripeManager::new(2, 3, 3).unwrap();

        // Count how many times each account is assigned parity over many stripes
        let mut parity_counts = vec![0usize; 3];

        // Parity is at index (N-1) in unrotated form
        // With rotation, it moves around
        for stripe_idx in 0..30 {
            let assignments = manager.get_assignments(stripe_idx);
            // In a 2+1 config, the last block (index 2) is parity
            // After rotation by stripe_idx, the parity position shifts
            // But we're tracking which *account* gets assigned to each position
            // The account at position 2 (the parity position in original layout)
            // rotates as well
            for (pos, &account) in assignments.iter().enumerate() {
                // Track which account gets the "parity position" in original layout
                if pos == 2 {
                    parity_counts[account as usize] += 1;
                }
            }
        }

        // Each account should get roughly equal parity assignments
        // With 30 stripes and 3 accounts, expect 10 each
        for (account, count) in parity_counts.iter().enumerate() {
            assert_eq!(
                *count, 10,
                "Account {} should have 10 parity assignments, got {}",
                account, count
            );
        }
    }

    #[test]
    fn test_reconstruct_with_invalid_block_index() {
        let manager = StripeManager::new(2, 3, 3).unwrap();

        let blocks: Vec<(u8, Vec<u8>)> = vec![
            (0, vec![1, 2, 3]),
            (99, vec![4, 5, 6]), // Invalid index
        ];

        assert!(manager.reconstruct(&blocks).is_err());
    }

    #[test]
    fn test_stripe_info_can_reconstruct() {
        let manager = StripeManager::new(2, 3, 3).unwrap();
        let data = b"Test data";
        let chunk_id = "test".to_string();

        let stripe = manager.create_stripe(chunk_id, data, 0).unwrap();

        // All blocks uploaded
        let all_message_ids: Vec<(u8, i32)> = vec![(0, 100), (1, 101), (2, 102)];
        let stripe_info = manager.to_stripe_info(&stripe, &all_message_ids);
        assert!(stripe_info.can_reconstruct());

        // Only K blocks uploaded (minimum)
        let min_message_ids: Vec<(u8, i32)> = vec![(0, 100), (1, 101)];
        let stripe_info = manager.to_stripe_info(&stripe, &min_message_ids);
        assert!(stripe_info.can_reconstruct());

        // Less than K blocks uploaded
        let insufficient_ids: Vec<(u8, i32)> = vec![(0, 100)];
        let stripe_info = manager.to_stripe_info(&stripe, &insufficient_ids);
        assert!(!stripe_info.can_reconstruct());
    }
}
