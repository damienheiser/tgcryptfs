# tgcryptfs Security Model

This document describes the security architecture, threat model, and cryptographic design of tgcryptfs.

## Security Goals

tgcryptfs is designed to achieve the following security properties:

1. **Confidentiality**: File contents, names, and directory structure are hidden from Telegram and network observers
2. **Integrity**: Any tampering with stored data is detected
3. **Authenticity**: Data can only be created by someone with the correct password
4. **Forward Secrecy**: Compromise of current keys doesn't expose historical data (with snapshots)

## Threat Model

### In Scope

| Threat | Mitigation |
|--------|------------|
| Telegram reading your data | All data encrypted before upload |
| Telegram modifying your data | AES-GCM authentication detects tampering |
| Network eavesdroppers | TLS to Telegram + our encryption layer |
| Local disk theft (cache) | Cache contains decrypted data (see limitations) |
| Password brute-forcing | Argon2id with high memory/time cost |

### Out of Scope

| Threat | Reason |
|--------|--------|
| Compromised local machine | If attacker has root, game over |
| Malicious tgcryptfs binary | Supply chain attacks not addressed |
| Side-channel attacks | Not hardened against timing/power analysis |
| Rubber-hose cryptanalysis | Can't help with physical coercion |

## Cryptographic Design

### Key Hierarchy

```
                    User Password
                          │
                          ▼
              ┌───────────────────────┐
              │      Argon2id         │
              │   (salt, params)      │
              └───────────────────────┘
                          │
                          ▼
                    Master Key (256 bits)
                          │
          ┌───────────────┼───────────────┐
          │               │               │
          ▼               ▼               ▼
    ┌──────────┐    ┌──────────┐    ┌──────────┐
    │ Metadata │    │ Chunk 1  │    │ Chunk N  │
    │   Key    │    │   Key    │    │   Key    │
    └──────────┘    └──────────┘    └──────────┘
```

### Key Derivation Function

**Algorithm**: Argon2id (winner of Password Hashing Competition)

**Parameters** (configurable):
- Memory: 64 MiB (default)
- Iterations: 3
- Parallelism: 4
- Output: 256 bits

**Why Argon2id?**
- Resistant to GPU/ASIC attacks (memory-hard)
- Resistant to side-channel attacks (data-independent)
- Modern, well-analyzed algorithm

**Salt**: 32 bytes, randomly generated on first initialization, stored in config

### Subkey Derivation

**Algorithm**: HKDF-SHA256

**Purpose-specific derivation**:
```
Metadata Key = HKDF(Master Key, salt, "tgcryptfs-metadata-v1")
Chunk Key    = HKDF(Master Key, salt, "tgcryptfs-chunk-v1:<chunk_id>")
```

**Why per-chunk keys?**
- Limits exposure if a single chunk key is compromised
- Enables future key rotation per-chunk
- Chunk ID (content hash) provides unique context

### Encryption

**Algorithm**: AES-256-GCM (AEAD)

**Parameters**:
- Key: 256 bits (from HKDF)
- Nonce: 96 bits, randomly generated per encryption
- Tag: 128 bits, appended to ciphertext

**Properties**:
- **Confidentiality**: AES in counter mode
- **Integrity**: GHASH polynomial authentication
- **Authenticity**: Verifies data came from key holder

**Nonce handling**:
- Fresh random nonce for every encryption
- Never reused (random 96-bit collision probability negligible)
- Stored with ciphertext

### Content Hashing

**Algorithm**: BLAKE3

**Usage**:
- Chunk identification (content-addressing)
- File integrity verification
- Deduplication detection

**Why BLAKE3?**
- Faster than SHA-256
- Cryptographically secure
- Parallelizable
- No length extension attacks

## Data Protection

### What's Encrypted

| Data | Encryption | Storage |
|------|------------|---------|
| File contents | AES-256-GCM per chunk | Telegram |
| Inode metadata | AES-256-GCM with metadata key | Local sled DB |
| Directory structure | Encoded in encrypted inodes | Local sled DB |
| Chunk manifest | Part of encrypted inode | Local sled DB |
| Snapshot data | AES-256-GCM with metadata key | Local (exportable) |
| Configuration | Plaintext (no secrets except salt) | Local JSON |

### What Telegram Sees

Telegram only receives:
- Encrypted blobs (random-looking bytes)
- File names like `tgfs_chunk_<random>` (no real filenames)
- File sizes (chunk sizes, not original file sizes)
- Upload/download patterns (timing metadata)

Telegram cannot determine:
- File contents
- Original file names
- Directory structure
- Which chunks belong to which files
- How many files you have

### Local Storage Security

**Metadata database** (sled):
- All values encrypted before storage
- Keys are plaintext (inode numbers, chunk IDs)
- Protected by filesystem permissions

**Cache directory**:
- Contains **decrypted** chunk data for performance
- Protected only by filesystem permissions
- Clear with `tgcryptfs cache --clear`

**Configuration**:
- Contains salt (not secret, but needed)
- Contains Telegram credentials (protect this file!)
- No encryption keys stored

## Authentication Flow

### Initial Setup
```
1. User provides password
2. Generate random 32-byte salt
3. Derive master key via Argon2id
4. Store salt in configuration (not the key!)
5. Initialize root inode with derived metadata key
```

### Mounting
```
1. User provides password
2. Load salt from configuration
3. Derive master key via Argon2id (same params)
4. Derive metadata key
5. Attempt to decrypt root inode
6. If decryption fails → wrong password
7. If successful → mount filesystem
```

### Password Verification

There's no stored "password hash" to verify against. Instead:
- Derive keys from provided password
- Attempt to decrypt existing metadata
- GCM authentication failure = wrong password

This provides implicit verification without storing password-equivalent data.

## Deduplication Security

### Content-Based Deduplication

Files are split into chunks, each identified by its BLAKE3 hash.

**Security consideration**: Identical content produces identical chunk IDs.

**Implications**:
- An attacker who knows plaintext can check if it exists
- Mitigated: Chunks are encrypted, so hash is of encrypted data
- Actually: We hash plaintext for dedup, then encrypt

**Current design**:
```
Chunk ID = BLAKE3(plaintext_chunk)
Stored   = Encrypt(plaintext_chunk, ChunkKey(chunk_id))
```

**Trade-off**:
- Pro: Deduplication across files and time
- Con: Potential for confirmation attacks on known plaintext

**Alternative** (not implemented):
```
Chunk ID = BLAKE3(encrypted_chunk)
```
This would prevent confirmation attacks but break cross-session deduplication.

## Known Limitations

### Cache Security

The local cache stores **decrypted** data for performance. This means:
- Cached data is readable by local processes
- Disk forensics could recover cached data
- Use full-disk encryption for defense in depth

**Mitigation**:
- Clear cache: `tgcryptfs cache --clear`
- Disable cache (not implemented, future feature)
- Use encrypted filesystem for cache directory

### Metadata Leakage

While file contents are encrypted, some metadata leaks:
- **Chunk count**: Reveals approximate file sizes
- **Access patterns**: Telegram sees which chunks are accessed when
- **Timing**: Operation timing could reveal activity patterns

### Single Password

All security derives from one password:
- Password compromise = total compromise
- No multi-user support
- No password recovery mechanism

### Trust in Telegram

We trust Telegram for:
- **Availability**: They could delete your data
- **Durability**: They could lose your data
- **Ordering**: Message IDs are trusted for chunk references

We don't trust Telegram for:
- Confidentiality (encrypted)
- Integrity (authenticated encryption)

## Security Recommendations

### Strong Password

Choose a strong, unique password:
- 16+ characters recommended
- Use a password manager
- Don't reuse from other services

### Local Security

Protect your local machine:
- Full-disk encryption
- Screen lock
- Secure configuration file permissions

### Backup Strategy

Consider that:
- Telegram could ban your account
- Telegram could lose data
- You could forget your password

Keep offline backups of critical data.

### Configuration Security

Protect `~/.config/tgcryptfs/config.json`:
```bash
chmod 600 ~/.config/tgcryptfs/config.json
```

This file contains your Telegram API credentials.

## Cryptographic Library Choices

| Purpose | Library | Rationale |
|---------|---------|-----------|
| Key derivation | argon2 | Pure Rust, well-audited |
| Encryption | ring | BoringSSL-backed, audited |
| HKDF | ring | Consistent with encryption |
| Hashing | blake3 | Fast, secure, Rust-native |
| Random | rand | ChaCha20-based, secure |

## Future Security Enhancements

### Planned

1. **Key rotation**: Ability to re-encrypt with new key
2. **Cache encryption**: Encrypt local cache
3. **Secure memory**: Use mlock for key material
4. **Yubikey support**: Hardware key derivation

### Considered

1. **Split knowledge**: Require multiple passwords
2. **Plausible deniability**: Hidden volumes
3. **Post-quantum**: Hybrid encryption schemes

## Reporting Security Issues

If you discover a security vulnerability:
1. Do NOT open a public issue
2. Email security@[project-domain] with details
3. Allow reasonable time for fix before disclosure
