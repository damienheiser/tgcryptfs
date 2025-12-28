# tgcryptfs Disaster Recovery Document

**Created:** 2025-12-28 10:27:02 UTC
**Version:** tgcryptfs v2.0 (0.1.0 binary)

---

## Encryption Keys

### CLOUDYDAY (Remote Server)
- **Machine:** cloudyday (hedon@cloudyday)
- **Key Location:** `/etc/tgcryptfs/encryption.key`
- **Key Value:** `BpXsLKWzfRcoUn9isIDGh/taelJ7M0AUaw2L2+O9fqw=`
- **Generated With:** `openssl rand -base64 32`

### PLEASURE (Local Mac)
- **Machine:** pleasure (local)
- **Key Location:** `~/.local/share/tgcryptfs/encryption.key`
- **Key Value:** `D0whcRGVW7xhRDHFddXaCaHUstkOvOdPPYsZpa4jCfs=`
- **Generated With:** `openssl rand -base64 32`

---

## Configuration Paths

### Cloudyday
- **Binary:** `/usr/local/bin/tgcryptfs`
- **Config:** `/etc/tgcryptfs/config.json`
- **Data Dir:** `/var/lib/tgcryptfs/`
- **Metadata DB:** `/var/lib/tgcryptfs/metadata.db`
- **Mount Point:** `/mnt/tgcryptfs`
- **Systemd Service:** `/etc/systemd/system/tgcryptfs.service`

### Pleasure
- **Binary:** `/usr/local/bin/tgcryptfs`
- **Config:** `~/.local/share/tgcryptfs/config.json`
- **Data Dir:** `~/.local/share/tgcryptfs/`
- **Metadata DB:** `~/.local/share/tgcryptfs/metadata.db`
- **Mount Point:** `~/mnt/tgcryptfs`

---

## Recovery Instructions

### From Total Loss (New Machine)

1. **Install tgcryptfs:**
   ```bash
   # Download from GitHub
   curl -L https://github.com/damienheiser/tgcryptfs/releases/download/v2.0/tgcryptfs-$(uname -s | tr '[:upper:]' '[:lower:]')-$(uname -m) -o tgcryptfs
   chmod +x tgcryptfs
   sudo mv tgcryptfs /usr/local/bin/
   ```

2. **Create key file:**
   ```bash
   mkdir -p ~/.local/share/tgcryptfs
   echo "YOUR_KEY_FROM_ABOVE" > ~/.local/share/tgcryptfs/encryption.key
   chmod 600 ~/.local/share/tgcryptfs/encryption.key
   ```

3. **Authenticate with Telegram:**
   ```bash
   tgcryptfs auth --phone +YOUR_PHONE
   ```

4. **Sync metadata from Telegram:**
   ```bash
   tgcryptfs sync --download
   ```

5. **Mount filesystem:**
   ```bash
   mkdir -p ~/mnt/tgcryptfs
   tgcryptfs mount ~/mnt/tgcryptfs --password-file ~/.local/share/tgcryptfs/encryption.key
   ```

### If Only Metadata Lost

The encrypted chunks are stored in Telegram Saved Messages. With the correct encryption key, you can rebuild the metadata:

```bash
tgcryptfs sync --rebuild
```

---

## GitHub Repository

- **URL:** https://github.com/damienheiser/tgcryptfs
- **Latest Release:** https://github.com/damienheiser/tgcryptfs/releases/tag/v2.0
- **Binaries Available:**
  - tgcryptfs-linux-x86_64
  - tgcryptfs-linux-aarch64
  - tgcryptfs-macos-aarch64

---

## HKDF Purpose Strings (Current)

- **Metadata:** `tgcryptfs-metadata-v1`
- **Chunks:** `tgcryptfs-chunk-v1:<chunk_id>`
- **Machine:** `tgcryptfs-machine-<uuid>`

---

## Important Notes

1. **NEVER lose these encryption keys** - data cannot be recovered without them
2. Telegram stores encrypted chunks but NOT the encryption key
3. The key is used to derive all other keys via HKDF
4. Argon2id parameters: 64MB memory, 3 iterations, 4 parallelism

---

## Telegram API Credentials

API credentials are embedded in the binary. If building from source:
- API_ID and API_HASH are required
- Get from https://my.telegram.org/apps

---

*This document contains sensitive cryptographic material. Store securely.*
