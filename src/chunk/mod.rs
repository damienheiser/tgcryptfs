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
