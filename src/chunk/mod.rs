//! Chunk management module
//!
//! Handles splitting files into chunks, content-addressable storage,
//! compression, and deduplication.

mod chunker;
mod compression;

pub use chunker::{Chunk, ChunkId, ChunkInfo, Chunker};
pub use compression::{compress, compress_or_original, decompress};

use serde::{Deserialize, Serialize};

/// Reference to a chunk stored remotely
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChunkRef {
    /// Content-based ID (BLAKE3 hash of encrypted content)
    pub id: ChunkId,
    /// Size of the encrypted chunk in bytes
    pub size: u64,
    /// Telegram message ID where this chunk is stored
    pub message_id: i32,
    /// Offset within file this chunk represents
    pub offset: u64,
    /// Original (unencrypted, uncompressed) size
    pub original_size: u64,
    /// Whether compression was applied
    pub compressed: bool,
}

/// Manifest describing all chunks of a file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkManifest {
    /// File version this manifest represents
    pub version: u64,
    /// Total file size (uncompressed)
    pub total_size: u64,
    /// Ordered list of chunk references
    pub chunks: Vec<ChunkRef>,
    /// BLAKE3 hash of the complete file content
    pub file_hash: String,
}

impl ChunkManifest {
    /// Create a new empty manifest
    pub fn new(version: u64) -> Self {
        ChunkManifest {
            version,
            total_size: 0,
            chunks: Vec::new(),
            file_hash: String::new(),
        }
    }

    /// Get the total stored size (after encryption/compression)
    pub fn stored_size(&self) -> u64 {
        self.chunks.iter().map(|c| c.size).sum()
    }

    /// Get the number of chunks
    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }

    /// Find the chunk containing a given offset
    pub fn chunk_at_offset(&self, offset: u64) -> Option<(usize, &ChunkRef)> {
        let mut current_offset = 0u64;
        for (idx, chunk) in self.chunks.iter().enumerate() {
            if offset >= current_offset && offset < current_offset + chunk.original_size {
                return Some((idx, chunk));
            }
            current_offset += chunk.original_size;
        }
        None
    }
}

/// Location of a single block within a stripe
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BlockLocation {
    /// Account ID (index in pool, 0-255)
    pub account_id: u8,
    /// Telegram message ID (None if not yet uploaded or unavailable)
    pub message_id: Option<i32>,
    /// Block index within stripe (0..N-1)
    pub block_index: u8,
    /// Upload timestamp (Unix seconds)
    pub uploaded_at: Option<i64>,
}

/// Stripe information for erasure-coded chunk
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StripeInfo {
    /// All blocks in this stripe (N total)
    pub blocks: Vec<BlockLocation>,
    /// Number of data blocks (K)
    pub data_count: u8,
    /// Number of parity blocks (N-K)
    pub parity_count: u8,
    /// Size of each block in bytes
    pub block_size: u64,
}

/// Reference to an erasure-coded chunk stored across multiple accounts
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErasureChunkRef {
    /// Content-based ID (BLAKE3 hash)
    pub id: ChunkId,
    /// Offset within file this chunk represents
    pub offset: u64,
    /// Original (unencrypted, uncompressed) size
    pub original_size: u64,
    /// Whether compression was applied before erasure coding
    pub compressed: bool,
    /// Stripe information with block locations
    pub stripe: StripeInfo,
    /// Version for rebuild tracking
    pub version: u64,
}

/// Manifest for erasure-coded files
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErasureChunkManifest {
    /// File version this manifest represents
    pub version: u64,
    /// Total file size (uncompressed)
    pub total_size: u64,
    /// Ordered list of erasure chunk references
    pub chunks: Vec<ErasureChunkRef>,
    /// BLAKE3 hash of the complete file content
    pub file_hash: String,
    /// Erasure coding parameters (K, N)
    pub data_chunks: u8,
    pub total_chunks: u8,
}

impl StripeInfo {
    /// Create a new stripe info
    pub fn new(data_count: u8, parity_count: u8, block_size: u64) -> Self {
        Self {
            blocks: Vec::with_capacity((data_count + parity_count) as usize),
            data_count,
            parity_count,
            block_size,
        }
    }

    /// Get total number of blocks (N)
    pub fn total_blocks(&self) -> u8 {
        self.data_count + self.parity_count
    }

    /// Count available (uploaded) blocks
    pub fn available_blocks(&self) -> usize {
        self.blocks.iter().filter(|b| b.message_id.is_some()).count()
    }

    /// Check if stripe can be reconstructed (>= K blocks available)
    pub fn can_reconstruct(&self) -> bool {
        self.available_blocks() >= self.data_count as usize
    }
}

impl ErasureChunkManifest {
    /// Create a new empty erasure manifest
    pub fn new(version: u64, data_chunks: u8, total_chunks: u8) -> Self {
        Self {
            version,
            total_size: 0,
            chunks: Vec::new(),
            file_hash: String::new(),
            data_chunks,
            total_chunks,
        }
    }
}
