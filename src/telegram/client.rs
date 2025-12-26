//! Telegram client implementation
//!
//! Uses grammers library to interact with Telegram API.
//! All data is uploaded to "Saved Messages" for private storage.

use crate::config::TelegramConfig;
use crate::error::{Error, Result};
use crate::telegram::rate_limit::{ExponentialBackoff, RateLimiter};
use crate::telegram::{CHUNK_FILE_PREFIX, METADATA_FILE_PREFIX};

use grammers_client::{Client, InputMessage, SignInError};
use grammers_mtsender::{SenderPool, SenderPoolHandle};
use grammers_session::storages::SqliteSession;
use grammers_session::defs::PeerRef;

use std::io::{BufRead, Cursor, Write};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tracing::{debug, error, info, warn};

/// Represents a message stored in Telegram
#[derive(Debug, Clone)]
pub struct TelegramMessage {
    /// Message ID
    pub id: i32,
    /// File name (if document)
    pub filename: Option<String>,
    /// File size in bytes
    pub size: u64,
    /// Message date
    pub date: i64,
}

/// Login token for completing sign-in
pub struct LoginToken {
    inner: grammers_client::types::LoginToken,
}

/// Password token for 2FA
pub struct PasswordToken {
    inner: grammers_client::types::PasswordToken,
}

impl PasswordToken {
    /// Get the password hint
    pub fn hint(&self) -> Option<&str> {
        self.inner.hint()
    }
}

/// Internal client state
struct ClientState {
    client: Client,
    #[allow(dead_code)]
    session: Arc<SqliteSession>,
    pool_handle: SenderPoolHandle,
    _pool_task: JoinHandle<()>,
}

/// Telegram backend for storing and retrieving chunks
///
/// This implementation uses the grammers crate to interact with Telegram.
pub struct TelegramBackend {
    /// Configuration
    config: TelegramConfig,
    /// Rate limiter for uploads
    upload_limiter: RateLimiter,
    /// Rate limiter for downloads
    download_limiter: RateLimiter,
    /// Client state (when connected)
    client_state: Arc<RwLock<Option<ClientState>>>,
}

impl TelegramBackend {
    /// Create a new Telegram backend
    pub fn new(config: TelegramConfig) -> Self {
        let upload_limiter = RateLimiter::new(
            config.max_concurrent_uploads,
            2.0, // 2 uploads per second max
        );
        let download_limiter = RateLimiter::new(
            config.max_concurrent_downloads,
            5.0, // 5 downloads per second max
        );

        TelegramBackend {
            config,
            upload_limiter,
            download_limiter,
            client_state: Arc::new(RwLock::new(None)),
        }
    }

    /// Check if connected
    pub fn is_connected(&self) -> bool {
        if let Ok(guard) = self.client_state.try_read() {
            guard.is_some()
        } else {
            false
        }
    }

    /// Get the session file path
    fn session_path(&self) -> PathBuf {
        let path = &self.config.session_file;
        if path.extension().is_none() {
            path.with_extension("session")
        } else {
            path.clone()
        }
    }

    /// Connect to Telegram
    pub async fn connect(&self) -> Result<()> {
        let session_path = self.session_path();

        // Ensure parent directory exists
        if let Some(parent) = session_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                Error::Configuration(format!("Failed to create session directory: {}", e))
            })?;
        }

        let session = Arc::new(
            SqliteSession::open(&session_path).map_err(|e| {
                Error::TelegramClient(format!("Failed to open session: {}", e))
            })?
        );

        let pool = SenderPool::new(Arc::clone(&session), self.config.api_id);
        let client = Client::new(&pool);
        let SenderPool { runner, handle, .. } = pool;

        let pool_task = tokio::spawn(runner.run());

        let state = ClientState {
            client,
            session,
            pool_handle: handle,
            _pool_task: pool_task,
        };

        *self.client_state.write().await = Some(state);
        info!("Connected to Telegram");
        Ok(())
    }

    /// Check if authorized
    pub async fn is_authorized(&self) -> Result<bool> {
        let state = self.client_state.read().await;
        let client_state = state.as_ref().ok_or_else(|| {
            Error::TelegramClient("Not connected".to_string())
        })?;

        client_state.client.is_authorized().await.map_err(|e| {
            Error::TelegramClient(format!("Failed to check authorization: {}", e))
        })
    }

    /// Request login code
    pub async fn request_login_code(&self, phone: &str) -> Result<LoginToken> {
        let state = self.client_state.read().await;
        let client_state = state.as_ref().ok_or_else(|| {
            Error::TelegramClient("Not connected".to_string())
        })?;

        let token = client_state.client
            .request_login_code(phone, &self.config.api_hash)
            .await
            .map_err(|e| Error::TelegramClient(format!("Failed to request login code: {}", e)))?;

        Ok(LoginToken { inner: token })
    }

    /// Sign in with code
    pub async fn sign_in(&self, token: &LoginToken, code: &str) -> Result<Option<PasswordToken>> {
        let state = self.client_state.read().await;
        let client_state = state.as_ref().ok_or_else(|| {
            Error::TelegramClient("Not connected".to_string())
        })?;

        match client_state.client.sign_in(&token.inner, code).await {
            Ok(_) => {
                info!("Successfully signed in");
                Ok(None)
            }
            Err(SignInError::PasswordRequired(password_token)) => {
                Ok(Some(PasswordToken { inner: password_token }))
            }
            Err(e) => Err(Error::TelegramClient(format!("Sign in failed: {}", e))),
        }
    }

    /// Check password for 2FA
    pub async fn check_password(&self, token: PasswordToken, password: &str) -> Result<()> {
        let state = self.client_state.read().await;
        let client_state = state.as_ref().ok_or_else(|| {
            Error::TelegramClient("Not connected".to_string())
        })?;

        client_state.client
            .check_password(token.inner, password)
            .await
            .map_err(|e| Error::TelegramClient(format!("Password check failed: {}", e)))?;

        info!("Successfully authenticated with 2FA");
        Ok(())
    }

    /// Bot sign in
    pub async fn bot_sign_in(&self, token: &str) -> Result<()> {
        let state = self.client_state.read().await;
        let client_state = state.as_ref().ok_or_else(|| {
            Error::TelegramClient("Not connected".to_string())
        })?;

        client_state.client
            .bot_sign_in(token, &self.config.api_hash)
            .await
            .map_err(|e| Error::TelegramClient(format!("Bot sign in failed: {}", e)))?;

        info!("Successfully signed in as bot");
        Ok(())
    }

    /// Get PeerRef for "Saved Messages" (self)
    #[allow(dead_code)]
    async fn get_self_peer(&self) -> Result<PeerRef> {
        let state = self.client_state.read().await;
        let client_state = state.as_ref().ok_or_else(|| {
            Error::TelegramClient("Not connected".to_string())
        })?;

        let me = client_state.client.get_me().await.map_err(|e| {
            Error::TelegramClient(format!("Failed to get self: {}", e))
        })?;

        // Convert to PeerRef via the raw tl type
        Ok(PeerRef::from(me.raw))
    }

    /// Upload a chunk to Saved Messages
    pub async fn upload_chunk(&self, chunk_id: &str, data: &[u8]) -> Result<i32> {
        let _permit = self.upload_limiter.acquire().await;

        let filename = format!("{}{}", CHUNK_FILE_PREFIX, chunk_id);
        debug!("Uploading chunk: {} ({} bytes)", filename, data.len());

        let mut backoff = ExponentialBackoff::new(
            self.config.retry_base_delay_ms,
            self.config.retry_attempts,
        );

        loop {
            match self.do_upload(&filename, data).await {
                Ok(msg_id) => {
                    debug!("Chunk {} uploaded as message {}", chunk_id, msg_id);
                    return Ok(msg_id);
                }
                Err(e) => {
                    if let Some(delay) = backoff.next_delay() {
                        warn!("Upload failed, retrying in {:?}: {}", delay, e);
                        tokio::time::sleep(delay).await;
                    } else {
                        error!("Upload failed after max retries: {}", e);
                        return Err(e);
                    }
                }
            }
        }
    }

    /// Internal upload implementation
    async fn do_upload(&self, filename: &str, data: &[u8]) -> Result<i32> {
        let state = self.client_state.read().await;
        let client_state = state.as_ref().ok_or_else(|| {
            Error::TelegramClient("Not connected".to_string())
        })?;

        let me = client_state.client.get_me().await.map_err(|e| {
            Error::TelegramClient(format!("Failed to get self: {}", e))
        })?;
        let peer = PeerRef::from(me.raw);

        // Upload file from memory using upload_stream
        let mut cursor = Cursor::new(data);
        let uploaded = client_state.client
            .upload_stream(&mut cursor, data.len(), filename.to_string())
            .await
            .map_err(|e| Error::TelegramUpload(format!("Failed to upload file: {}", e)))?;

        let message = InputMessage::new()
            .document(uploaded);

        let sent = client_state.client
            .send_message(peer, message)
            .await
            .map_err(|e| Error::TelegramUpload(format!("Failed to send message: {}", e)))?;

        Ok(sent.id())
    }

    /// Download a chunk by message ID
    pub async fn download_chunk(&self, message_id: i32) -> Result<Vec<u8>> {
        let _permit = self.download_limiter.acquire().await;

        debug!("Downloading chunk from message {}", message_id);

        let mut backoff = ExponentialBackoff::new(
            self.config.retry_base_delay_ms,
            self.config.retry_attempts,
        );

        loop {
            match self.do_download(message_id).await {
                Ok(data) => {
                    debug!("Downloaded {} bytes from message {}", data.len(), message_id);
                    return Ok(data);
                }
                Err(e) => {
                    if let Some(delay) = backoff.next_delay() {
                        warn!("Download failed, retrying in {:?}: {}", delay, e);
                        tokio::time::sleep(delay).await;
                    } else {
                        error!("Download failed after max retries: {}", e);
                        return Err(e);
                    }
                }
            }
        }
    }

    /// Internal download implementation
    async fn do_download(&self, message_id: i32) -> Result<Vec<u8>> {
        let state = self.client_state.read().await;
        let client_state = state.as_ref().ok_or_else(|| {
            Error::TelegramClient("Not connected".to_string())
        })?;

        let me = client_state.client.get_me().await.map_err(|e| {
            Error::TelegramClient(format!("Failed to get self: {}", e))
        })?;
        let peer = PeerRef::from(me.raw);

        // Get the message
        let messages = client_state.client
            .get_messages_by_id(peer, &[message_id])
            .await
            .map_err(|e| Error::TelegramDownload(format!("Failed to get message: {}", e)))?;

        let message = messages.into_iter().next().flatten().ok_or_else(|| {
            Error::TelegramDownload(format!("Message {} not found", message_id))
        })?;

        let media = message.media().ok_or_else(|| {
            Error::TelegramDownload(format!("Message {} has no media", message_id))
        })?;

        // Download to memory
        let mut data = Vec::new();
        let mut download = client_state.client.iter_download(&media);

        while let Some(chunk) = download.next().await.map_err(|e| {
            Error::TelegramDownload(format!("Failed to download chunk: {}", e))
        })? {
            data.extend_from_slice(&chunk);
        }

        Ok(data)
    }

    /// Delete a message by ID
    pub async fn delete_message(&self, message_id: i32) -> Result<()> {
        let state = self.client_state.read().await;
        let client_state = state.as_ref().ok_or_else(|| {
            Error::TelegramClient("Not connected".to_string())
        })?;

        let me = client_state.client.get_me().await.map_err(|e| {
            Error::TelegramClient(format!("Failed to get self: {}", e))
        })?;
        let peer = PeerRef::from(me.raw);

        client_state.client
            .delete_messages(peer, &[message_id])
            .await
            .map_err(|e| Error::TelegramClient(format!("Failed to delete message: {}", e)))?;

        debug!("Deleted message {}", message_id);
        Ok(())
    }

    /// List all chunk messages in Saved Messages
    pub async fn list_chunks(&self) -> Result<Vec<TelegramMessage>> {
        let state = self.client_state.read().await;
        let client_state = state.as_ref().ok_or_else(|| {
            Error::TelegramClient("Not connected".to_string())
        })?;

        let me = client_state.client.get_me().await.map_err(|e| {
            Error::TelegramClient(format!("Failed to get self: {}", e))
        })?;
        let peer = PeerRef::from(me.raw);

        let mut messages = Vec::new();
        let mut iter = client_state.client.iter_messages(peer);

        while let Some(msg) = iter.next().await.map_err(|e| {
            Error::TelegramClient(format!("Failed to iterate messages: {}", e))
        })? {
            if let Some(media) = msg.media() {
                // Check if it's a document with our prefix
                if let grammers_client::types::Media::Document(doc) = media {
                    let name = doc.name();
                    if name.starts_with(CHUNK_FILE_PREFIX) || name.starts_with(METADATA_FILE_PREFIX) {
                        messages.push(TelegramMessage {
                            id: msg.id(),
                            filename: Some(name.to_string()),
                            size: doc.size() as u64,
                            date: msg.date().timestamp(),
                        });
                    }
                }
            }
        }

        Ok(messages)
    }

    /// Upload metadata to Saved Messages
    pub async fn upload_metadata(&self, name: &str, data: &[u8]) -> Result<i32> {
        let filename = format!("{}{}", METADATA_FILE_PREFIX, name);
        self.do_upload(&filename, data).await
    }

    /// Disconnect from Telegram
    pub async fn disconnect(&self) {
        let mut state = self.client_state.write().await;
        if let Some(client_state) = state.take() {
            client_state.pool_handle.quit();
            info!("Disconnected from Telegram");
        }
    }
}

/// Prompt for input (used during interactive auth)
#[allow(dead_code)]
pub fn prompt(message: &str) -> std::io::Result<String> {
    let stdout = std::io::stdout();
    let mut stdout = stdout.lock();
    stdout.write_all(message.as_bytes())?;
    stdout.flush()?;

    let stdin = std::io::stdin();
    let mut stdin = stdin.lock();

    let mut line = String::new();
    stdin.read_line(&mut line)?;
    Ok(line.trim().to_string())
}

/// Prompt for password (hides input)
#[allow(dead_code)]
pub fn prompt_password(message: &str) -> std::io::Result<String> {
    rpassword::prompt_password(message)
}
