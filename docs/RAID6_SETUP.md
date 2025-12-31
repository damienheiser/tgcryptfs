# RAID6 Setup Guide for tgcryptfs

This guide covers setting up RAID6-style erasure coding in tgcryptfs v3.1.0+ using multiple Telegram accounts for data redundancy.

## Overview

tgcryptfs uses Reed-Solomon erasure coding to distribute data across multiple Telegram accounts. This provides:

- **Data Redundancy**: Survive account failures without data loss
- **RAID5**: Tolerate 1 account failure (N-1 data, 1 parity)
- **RAID6**: Tolerate 2 account failures (N-2 data, 2 parity)
- **Custom**: Configure any K-of-N scheme

## Prerequisites

### 1. Number of Telegram Accounts Required

| Mode | Minimum Accounts | Recommended | Fault Tolerance |
|------|------------------|-------------|-----------------|
| RAID5 | 2 | 3-4 | 1 account |
| RAID6 | 3 | 4-5 | 2 accounts |
| Custom | K+1 | K+2 | N-K accounts |

**For RAID6 with 2-account fault tolerance:**
- **Minimum**: 4 accounts (2 data + 2 parity)
- **Recommended**: 5+ accounts (3+ data + 2 parity)

### 2. Creating Additional Telegram Accounts

Each Telegram account requires a unique phone number. Options include:

1. **Primary phone number** - Your main mobile number
2. **Secondary SIM cards** - Additional mobile numbers
3. **VoIP numbers** - Google Voice, TextNow, etc. (may have restrictions)
4. **Family/friend numbers** - With their permission for the initial verification

**Important**: Each account needs its own phone number for initial authentication. Telegram sends a verification code via SMS or the existing Telegram app.

### 3. API Credentials

Each Telegram account needs API credentials from [my.telegram.org](https://my.telegram.org):

1. Visit [https://my.telegram.org](https://my.telegram.org)
2. Log in with the phone number for each account
3. Click "API development tools"
4. Create a new application (any name works)
5. Note the `api_id` (integer) and `api_hash` (string)

**Tip**: You can reuse the same api_id/api_hash across multiple accounts if you prefer, but having separate credentials provides better isolation.

## Step-by-Step Setup

### Step 1: Add Accounts to the Pool

For each Telegram account, run:

```bash
# Add first account
tgcryptfs raid add-account \
    --api-id 12345678 \
    --api-hash "your_api_hash_here" \
    --session-file ~/.config/tgcryptfs/sessions/account0.session \
    --phone "+1234567890"

# Add second account
tgcryptfs raid add-account \
    --api-id 12345679 \
    --api-hash "another_api_hash" \
    --session-file ~/.config/tgcryptfs/sessions/account1.session \
    --phone "+1234567891"

# Add more accounts as needed...
tgcryptfs raid add-account \
    --api-id 12345680 \
    --api-hash "third_api_hash" \
    --session-file ~/.config/tgcryptfs/sessions/account2.session \
    --phone "+1234567892"

# For RAID6, add at least 4 accounts (recommended 5+)
tgcryptfs raid add-account \
    --api-id 12345681 \
    --api-hash "fourth_api_hash" \
    --session-file ~/.config/tgcryptfs/sessions/account3.session \
    --phone "+1234567893"
```

Each `add-account` command:
- Assigns an account ID (0, 1, 2, ...)
- Stores credentials in your config
- Automatically updates the erasure coding parameters

### Step 2: Authenticate Each Account

Authenticate each account with Telegram:

```bash
# Authenticate account 0
tgcryptfs auth --phone "+1234567890"
# Enter the code sent to this phone

# Authenticate account 1
tgcryptfs auth --phone "+1234567891"
# Enter the code sent to this phone

# Repeat for all accounts...
```

If an account has 2FA enabled, you'll be prompted for the password after entering the code.

### Step 3: Verify Pool Status

Check that all accounts are configured and connected:

```bash
tgcryptfs raid status
```

Expected output for a 5-account RAID6 setup:

```
RAID Array Status
=================

Configuration:
  Data chunks (K): 3
  Total chunks (N): 5
  Fault tolerance: 2 account(s)
  Preset: raid6

Accounts (5):
  [0] +1234567890 - ~/.config/tgcryptfs/sessions/account0.session (enabled)
  [1] +1234567891 - ~/.config/tgcryptfs/sessions/account1.session (enabled)
  [2] +1234567892 - ~/.config/tgcryptfs/sessions/account2.session (enabled)
  [3] +1234567893 - ~/.config/tgcryptfs/sessions/account3.session (enabled)
  [4] +1234567894 - ~/.config/tgcryptfs/sessions/account4.session (enabled)

Array Status: Healthy
Healthy accounts: 5/5
  All accounts operational, full redundancy.

Account Health:
  [0] Healthy - 0 ops, 0.0% error rate
  [1] Healthy - 0 ops, 0.0% error rate
  [2] Healthy - 0 ops, 0.0% error rate
  [3] Healthy - 0 ops, 0.0% error rate
  [4] Healthy - 0 ops, 0.0% error rate
```

### Step 4: Configure RAID Level

The erasure coding parameters are automatically set when you add accounts:

| Accounts | Default Mode | K (data) | N (total) | Fault Tolerance |
|----------|--------------|----------|-----------|-----------------|
| 2 | RAID5 | 1 | 2 | 1 |
| 3 | RAID5 | 2 | 3 | 1 |
| 4 | RAID5 | 3 | 4 | 1 |
| 5 | RAID5 | 4 | 5 | 1 |

To explicitly set RAID6 mode, edit your config file (`~/.config/tgcryptfs/config.yaml`):

```yaml
pool:
  erasure:
    enabled: true
    preset: raid6
    data_chunks: 3    # K - for 5 accounts
    total_chunks: 5   # N - equals account count
  accounts:
    # ... your accounts ...
```

**RAID6 configurations by account count:**

| Accounts | K (data) | N (total) | Formula |
|----------|----------|-----------|---------|
| 4 | 2 | 4 | N-2 data, 2 parity |
| 5 | 3 | 5 | N-2 data, 2 parity |
| 6 | 4 | 6 | N-2 data, 2 parity |
| 7 | 5 | 7 | N-2 data, 2 parity |

## Migration from Single Account

If you have existing data stored on a single Telegram account, you can migrate to erasure-coded storage.

### Step 1: Perform a Dry Run

Always test the migration first:

```bash
tgcryptfs raid migrate-to-erasure --dry-run
```

This shows what would happen without modifying any data:

```
DRY RUN: Would migrate file
  inode: 42
  path: /documents/report.pdf
  chunks: 3

Migration to erasure coding (DRY RUN):
  1. Read existing chunk manifests from metadata
  2. For each chunk stored on single account:
     - Download the chunk
     - Encode into 5 blocks using Reed-Solomon
     - Upload blocks to 5 accounts in parallel
     - Update manifest with ErasureChunkRef
```

### Step 2: Run the Migration

Once satisfied with the dry run:

```bash
tgcryptfs raid migrate-to-erasure
```

With optional flags:

```bash
# Delete old single-account messages after successful migration
tgcryptfs raid migrate-to-erasure --delete-old
```

### What Happens to Existing Data

During migration:

1. **Download**: Each chunk is downloaded from your original single account
2. **Encode**: Data is encoded into N blocks using Reed-Solomon
3. **Upload**: Blocks are uploaded to all accounts in parallel
4. **Update**: Metadata is updated with new `ErasureChunkRef` entries
5. **Verify**: Optional verification ensures data can be reconstructed
6. **Cleanup**: Old messages can optionally be deleted

**Migration is resumable**: If interrupted, re-running the command will continue where it left off. Already-migrated files are tracked and skipped.

### Migration Progress

The migration shows progress for each file:

```
Starting migration
  files: 150
  chunks: 2340
  dry_run: false

Starting file migration
  inode: 42
  path: /documents/report.pdf
  chunks: 3

Chunk migration complete
  inode: 42
  chunk: 0
  total: 3

Chunk migration complete
  inode: 42
  chunk: 1
  total: 3

Chunk migration complete
  inode: 42
  chunk: 2
  total: 3

File migration complete
  inode: 42
  path: /documents/report.pdf

Migration progress
  files: 1/150
  chunks: 3/2340
  percent: 0.7%
  bytes: 157286400
```

## Configuration Options

### RAID5 vs RAID6

**RAID5** (Default with 2+ accounts):
- K = N - 1 (one parity shard)
- Can survive 1 account failure
- Better storage efficiency

**RAID6** (Recommended for critical data):
- K = N - 2 (two parity shards)
- Can survive 2 simultaneous account failures
- Higher redundancy

### Preset Configurations

Set in config file or determined automatically:

```yaml
pool:
  erasure:
    preset: raid5   # Options: raid5, raid6, custom
```

For custom configurations:

```yaml
pool:
  erasure:
    preset: custom
    data_chunks: 4    # K
    total_chunks: 7   # N
```

This allows 3 account failures (7-4=3 parity shards).

### Pool Configuration Options

```yaml
pool:
  # Erasure coding settings
  erasure:
    enabled: true
    preset: raid6
    data_chunks: 3
    total_chunks: 5

  # Performance tuning
  max_concurrent_uploads: 6      # Parallel uploads across all accounts
  max_concurrent_downloads: 10   # Parallel downloads across all accounts
  retry_attempts: 3              # Retries per failed operation
  health_check_interval_secs: 300  # Health check every 5 minutes

  # Account configurations
  accounts:
    - account_id: 0
      api_id: 12345678
      api_hash: "your_hash"
      phone: "+1234567890"
      session_file: ~/.config/tgcryptfs/sessions/account0.session
      priority: 100              # Higher = preferred for uploads
      enabled: true
    # ... more accounts
```

### Account Priority

Accounts with higher priority values are preferred for chunk uploads when the array is healthy. This is useful if some accounts have better connectivity or fewer rate limits.

## Operations

### Monitoring Health

Check the array status regularly:

```bash
tgcryptfs raid status
```

Array status meanings:

| Status | Description | Action |
|--------|-------------|--------|
| **Healthy** | All accounts operational, full redundancy | None needed |
| **Degraded** | Operating with reduced redundancy | Investigate failed account(s) |
| **Failed** | Not enough accounts available (< K) | Immediate action required |
| **Rebuilding** | Rebuild in progress | Wait for completion |

Account status meanings:

| Status | Description |
|--------|-------------|
| **Healthy** | Account operational, < 10% error rate |
| **Degraded** | Account functional but high error rate (> 10%) |
| **Unavailable** | Account failed (3+ consecutive failures) |
| **Rebuilding** | Account being rebuilt |

### Rebuilding a Failed Account

When an account fails or is replaced:

```bash
# Rebuild data for account 2
tgcryptfs raid rebuild 2
```

The rebuild process:

1. Marks the account as "Rebuilding"
2. For each stripe with a block on this account:
   - Downloads K blocks from healthy accounts
   - Reconstructs the missing block using Reed-Solomon
   - Re-uploads to the target account
3. Marks the account as "Healthy" when complete

**Requirements**: At least K healthy accounts must be available to rebuild.

### Scrubbing for Integrity

Periodically verify all data integrity:

```bash
# Verify all stripes
tgcryptfs raid scrub

# Verify and repair any issues found
tgcryptfs raid scrub --repair
```

The scrub process:

1. Iterates through all stored stripes
2. Downloads all blocks for each stripe
3. Verifies Reed-Solomon decoding succeeds
4. Reports any inconsistencies
5. With `--repair`: Re-uploads missing/corrupted blocks

**Recommendation**: Run scrub monthly or after any account issues.

## Troubleshooting

### Account Rate-Limited

**Symptoms**:
- Upload/download failures
- "Too many requests" errors
- Account marked as Degraded

**Solutions**:

1. **Wait**: Telegram rate limits are temporary (usually minutes to hours)
2. **Reduce concurrency**: Lower `max_concurrent_uploads` and `max_concurrent_downloads`
3. **Stagger operations**: Avoid large batch uploads during peak times
4. **Add accounts**: More accounts = lower load per account

### Account Banned or Suspended

**Symptoms**:
- Persistent authentication failures
- Account marked as Unavailable
- "USER_DEACTIVATED" errors

**Solutions**:

1. **Check account status**: Log into the account via official Telegram app
2. **If recoverable**: Re-authenticate with `tgcryptfs auth`
3. **If permanently banned**: Add a new account and rebuild:
   ```bash
   # Add replacement account
   tgcryptfs raid add-account --api-id ... --api-hash ... --session-file ... --phone ...

   # Rebuild the failed account's data onto the new account
   tgcryptfs raid rebuild <old_account_id>
   ```

### Recovery Scenarios

**Scenario 1: Single Account Failure (RAID6)**

With 5 accounts and RAID6 (K=3, N=5):
- Array status: Degraded
- Data access: Unaffected (3 healthy accounts >= K)
- Action: Rebuild or replace account when convenient

**Scenario 2: Two Account Failures (RAID6)**

With 5 accounts and RAID6 (K=3, N=5):
- Array status: Degraded (critical)
- Data access: Still functional but no redundancy
- Action: Immediate rebuild/replacement required

**Scenario 3: Three+ Account Failures (RAID6)**

With 5 accounts and RAID6 (K=3, N=5):
- Array status: Failed
- Data access: **DATA LOSS** - cannot reconstruct
- Prevention: Monitor health, maintain spare accounts

### Data Recovery Priority

If the array enters Failed state:

1. **Do not write new data** - may corrupt metadata
2. **Identify recoverable accounts** - check each account's status
3. **Recover what's possible** - stripes with >= K available blocks
4. **Backup recoverable data** - to a separate location
5. **Restore failed accounts** - add new accounts, rebuild

### Performance Tuning

**Slow uploads/downloads**:
- Increase `max_concurrent_uploads` / `max_concurrent_downloads`
- Add more accounts to spread load
- Use accounts with better network connectivity

**High error rates**:
- Decrease concurrency to reduce rate limiting
- Increase `retry_attempts`
- Check account health individually

## Best Practices

1. **Use RAID6 for critical data** - The extra parity is worth it
2. **Maintain N+1 accounts** - Have a spare ready for quick replacement
3. **Monitor health regularly** - Run `tgcryptfs raid status` weekly
4. **Scrub periodically** - Monthly scrub catches silent corruption
5. **Keep credentials backed up** - Store api_id/api_hash and session files securely
6. **Test recovery** - Periodically verify you can recover from account failures
7. **Document your setup** - Record which phone numbers map to which accounts

## Example: Complete RAID6 Setup

Here's a complete example setting up a 5-account RAID6 array:

```bash
# Step 1: Initialize tgcryptfs
tgcryptfs init --api-id 12345678 --api-hash "your_primary_hash"

# Step 2: Add all accounts
for i in 0 1 2 3 4; do
  tgcryptfs raid add-account \
    --api-id "$(cat ~/accounts/account${i}_id)" \
    --api-hash "$(cat ~/accounts/account${i}_hash)" \
    --session-file ~/.config/tgcryptfs/sessions/account${i}.session \
    --phone "$(cat ~/accounts/account${i}_phone)"
done

# Step 3: Authenticate all accounts
for phone in $(cat ~/accounts/phones.txt); do
  echo "Authenticating $phone..."
  tgcryptfs auth --phone "$phone"
done

# Step 4: Verify setup
tgcryptfs raid status

# Step 5: Mount and use
tgcryptfs mount /mnt/tgcryptfs

# Step 6: If migrating from single account
tgcryptfs raid migrate-to-erasure --dry-run
tgcryptfs raid migrate-to-erasure
```

## Related Documentation

- [CONFIG_V2.md](CONFIG_V2.md) - Full configuration reference
- [DISTRIBUTED_ARCHITECTURE.md](DISTRIBUTED_ARCHITECTURE.md) - Multi-machine sync
- [SECURITY.md](SECURITY.md) - Encryption and security details
- [RECOVERY.md](RECOVERY.md) - Data recovery procedures
