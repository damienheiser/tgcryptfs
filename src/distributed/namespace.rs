//! Namespace system for tgcryptfs
//!
//! Namespaces provide logical isolation of filesystems on the same Telegram account.
//! Multiple namespaces can coexist without interfering with each other.

use crate::crypto::KEY_SIZE;
use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

/// Namespace types define how a namespace is used
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum NamespaceType {
    /// Private to this machine only
    Standalone,

    /// Shared with master-replica model
    MasterReplica {
        /// Master machine that can write
        master_id: Uuid,
        /// Replica machines that can only read
        replicas: Vec<Uuid>,
    },

    /// Shared with CRDT consensus for full read/write
    Distributed {
        /// Cluster identifier
        cluster_id: String,
        /// Member machines in the cluster
        members: Vec<Uuid>,
    },
}

/// Access control subject
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum AccessSubject {
    /// Specific machine
    Machine(Uuid),
    /// Group of machines
    MachineGroup(String),
    /// Any authenticated machine
    AnyAuthenticated,
    /// Public access (anyone)
    Public,
}

/// Permission flags
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Permissions {
    /// Read access
    pub read: bool,
    /// Write access
    pub write: bool,
    /// Delete access
    pub delete: bool,
    /// Admin access (can modify ACLs)
    pub admin: bool,
}

impl Permissions {
    /// Create read-only permissions
    pub fn read_only() -> Self {
        Self {
            read: true,
            write: false,
            delete: false,
            admin: false,
        }
    }

    /// Create read-write permissions
    pub fn read_write() -> Self {
        Self {
            read: true,
            write: true,
            delete: true,
            admin: false,
        }
    }

    /// Create full permissions (including admin)
    pub fn full() -> Self {
        Self {
            read: true,
            write: true,
            delete: true,
            admin: true,
        }
    }
}

/// Access control rule
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessRule {
    /// Who this rule applies to
    pub subject: AccessSubject,
    /// What permissions are granted
    pub permissions: Permissions,
    /// Path pattern (glob-style)
    pub path_pattern: String,
}

impl AccessRule {
    /// Create a new access rule
    pub fn new(subject: AccessSubject, permissions: Permissions, path_pattern: String) -> Self {
        Self {
            subject,
            permissions,
            path_pattern,
        }
    }
}

/// Namespace represents an isolated filesystem
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Namespace {
    /// Namespace identifier (must be unique)
    pub namespace_id: String,

    /// Namespace type
    pub namespace_type: NamespaceType,

    /// Encryption key for this namespace
    pub encryption_key: [u8; KEY_SIZE],

    /// Access control list
    pub acl: Vec<AccessRule>,

    /// Telegram message prefix for this namespace
    /// Format: tgfs:{namespace_id}:{type}:{id}
    pub telegram_prefix: String,

    /// Description
    pub description: Option<String>,
}

impl Namespace {
    /// Create a new namespace
    pub fn new(
        namespace_id: String,
        namespace_type: NamespaceType,
        encryption_key: [u8; KEY_SIZE],
    ) -> Self {
        let telegram_prefix = format!("tgfs:{}", namespace_id);

        Self {
            namespace_id: namespace_id.clone(),
            namespace_type,
            encryption_key,
            acl: Vec::new(),
            telegram_prefix,
            description: None,
        }
    }

    /// Create a standalone namespace
    pub fn standalone(namespace_id: String, encryption_key: [u8; KEY_SIZE]) -> Self {
        Self::new(namespace_id, NamespaceType::Standalone, encryption_key)
    }

    /// Create a master-replica namespace
    pub fn master_replica(
        namespace_id: String,
        encryption_key: [u8; KEY_SIZE],
        master_id: Uuid,
        replicas: Vec<Uuid>,
    ) -> Self {
        Self::new(
            namespace_id,
            NamespaceType::MasterReplica {
                master_id,
                replicas,
            },
            encryption_key,
        )
    }

    /// Create a distributed namespace
    pub fn distributed(
        namespace_id: String,
        encryption_key: [u8; KEY_SIZE],
        cluster_id: String,
        members: Vec<Uuid>,
    ) -> Self {
        Self::new(
            namespace_id,
            NamespaceType::Distributed {
                cluster_id,
                members,
            },
            encryption_key,
        )
    }

    /// Add an access rule
    pub fn add_rule(&mut self, rule: AccessRule) {
        self.acl.push(rule);
    }

    /// Check if a machine has permission for a path
    pub fn check_permission(
        &self,
        machine_id: &Uuid,
        path: &str,
        required_permission: PermissionType,
    ) -> bool {
        for rule in &self.acl {
            // Check if subject matches
            let subject_matches = match &rule.subject {
                AccessSubject::Machine(id) => id == machine_id,
                AccessSubject::AnyAuthenticated => true,
                AccessSubject::Public => true,
                AccessSubject::MachineGroup(_) => false, // TODO: implement groups
            };

            if !subject_matches {
                continue;
            }

            // Check if path matches (simple prefix match for now)
            if !path.starts_with(&rule.path_pattern) && rule.path_pattern != "*" {
                continue;
            }

            // Check if permission is granted
            let permission_granted = match required_permission {
                PermissionType::Read => rule.permissions.read,
                PermissionType::Write => rule.permissions.write,
                PermissionType::Delete => rule.permissions.delete,
                PermissionType::Admin => rule.permissions.admin,
            };

            if permission_granted {
                return true;
            }
        }

        false
    }

    /// Generate a Telegram message caption for this namespace
    pub fn telegram_caption(&self, msg_type: &str, id: &str) -> String {
        format!("{}:{}:{}", self.telegram_prefix, msg_type, id)
    }

    /// Get the storage key prefix for this namespace
    pub fn storage_prefix(&self) -> String {
        format!("ns:{}:", self.namespace_id)
    }

    /// Set description
    pub fn with_description(mut self, description: String) -> Self {
        self.description = Some(description);
        self
    }
}

/// Permission types for access control
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PermissionType {
    Read,
    Write,
    Delete,
    Admin,
}

/// Namespace manager manages multiple namespaces
pub struct NamespaceManager {
    /// Namespaces keyed by namespace_id
    namespaces: HashMap<String, Arc<Namespace>>,

    /// Default namespace (used if none specified)
    default_namespace: String,
}

impl NamespaceManager {
    /// Create a new namespace manager
    pub fn new(default_namespace: String) -> Self {
        Self {
            namespaces: HashMap::new(),
            default_namespace,
        }
    }

    /// Add a namespace
    pub fn add_namespace(&mut self, namespace: Namespace) -> Result<()> {
        let id = namespace.namespace_id.clone();

        if self.namespaces.contains_key(&id) {
            return Err(Error::AlreadyExists(format!("namespace: {}", id)));
        }

        self.namespaces.insert(id, Arc::new(namespace));
        Ok(())
    }

    /// Get a namespace by ID
    pub fn get_namespace(&self, namespace_id: &str) -> Result<Arc<Namespace>> {
        self.namespaces
            .get(namespace_id)
            .cloned()
            .ok_or_else(|| Error::Config(format!("namespace not found: {}", namespace_id)))
    }

    /// Get the default namespace
    pub fn get_default_namespace(&self) -> Result<Arc<Namespace>> {
        self.get_namespace(&self.default_namespace)
    }

    /// Remove a namespace
    pub fn remove_namespace(&mut self, namespace_id: &str) -> Result<()> {
        if namespace_id == self.default_namespace {
            return Err(Error::InvalidConfig(
                "cannot remove default namespace".to_string(),
            ));
        }

        self.namespaces
            .remove(namespace_id)
            .ok_or_else(|| Error::Config(format!("namespace not found: {}", namespace_id)))?;

        Ok(())
    }

    /// List all namespace IDs
    pub fn list_namespaces(&self) -> Vec<String> {
        self.namespaces.keys().cloned().collect()
    }

    /// Get namespace count
    pub fn namespace_count(&self) -> usize {
        self.namespaces.len()
    }

    /// Route a Telegram message caption to the correct namespace
    pub fn route_telegram_message(&self, caption: &str) -> Result<(Arc<Namespace>, String, String)> {
        // Parse caption: tgfs:{namespace}:{type}:{id}
        if !caption.starts_with("tgfs:") {
            return Err(Error::Config(format!("invalid message caption: {}", caption)));
        }

        let parts: Vec<&str> = caption[5..].splitn(3, ':').collect();
        if parts.len() != 3 {
            return Err(Error::Config(format!(
                "invalid message caption format: {}",
                caption
            )));
        }

        let namespace_id = parts[0];
        let msg_type = parts[1].to_string();
        let msg_id = parts[2].to_string();

        let namespace = self.get_namespace(namespace_id)?;
        Ok((namespace, msg_type, msg_id))
    }

    /// Check if a namespace exists
    pub fn has_namespace(&self, namespace_id: &str) -> bool {
        self.namespaces.contains_key(namespace_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key() -> [u8; KEY_SIZE] {
        [0u8; KEY_SIZE]
    }

    #[test]
    fn test_create_namespace() {
        let ns = Namespace::standalone("test".to_string(), test_key());
        assert_eq!(ns.namespace_id, "test");
        assert_eq!(ns.telegram_prefix, "tgfs:test");
        assert_eq!(ns.storage_prefix(), "ns:test:");
    }

    #[test]
    fn test_telegram_caption() {
        let ns = Namespace::standalone("myns".to_string(), test_key());
        let caption = ns.telegram_caption("chunk", "abc123");
        assert_eq!(caption, "tgfs:myns:chunk:abc123");
    }

    #[test]
    fn test_namespace_manager() {
        let mut mgr = NamespaceManager::new("default".to_string());

        let ns1 = Namespace::standalone("default".to_string(), test_key());
        let ns2 = Namespace::standalone("backup".to_string(), test_key());

        mgr.add_namespace(ns1).unwrap();
        mgr.add_namespace(ns2).unwrap();

        assert_eq!(mgr.namespace_count(), 2);
        assert!(mgr.has_namespace("default"));
        assert!(mgr.has_namespace("backup"));

        let default = mgr.get_default_namespace().unwrap();
        assert_eq!(default.namespace_id, "default");
    }

    #[test]
    fn test_route_telegram_message() {
        let mut mgr = NamespaceManager::new("default".to_string());
        let ns = Namespace::standalone("test".to_string(), test_key());
        mgr.add_namespace(ns).unwrap();

        let (namespace, msg_type, msg_id) =
            mgr.route_telegram_message("tgfs:test:chunk:abc123").unwrap();

        assert_eq!(namespace.namespace_id, "test");
        assert_eq!(msg_type, "chunk");
        assert_eq!(msg_id, "abc123");
    }

    #[test]
    fn test_permissions() {
        let mut ns = Namespace::standalone("test".to_string(), test_key());

        let machine_id = Uuid::new_v4();
        let rule = AccessRule::new(
            AccessSubject::Machine(machine_id),
            Permissions::read_write(),
            "*".to_string(),
        );

        ns.add_rule(rule);

        assert!(ns.check_permission(&machine_id, "/any/path", PermissionType::Read));
        assert!(ns.check_permission(&machine_id, "/any/path", PermissionType::Write));
        assert!(!ns.check_permission(&machine_id, "/any/path", PermissionType::Admin));
    }

    #[test]
    fn test_namespace_types() {
        let master_id = Uuid::new_v4();
        let replica1 = Uuid::new_v4();
        let replica2 = Uuid::new_v4();

        let ns = Namespace::master_replica(
            "shared".to_string(),
            test_key(),
            master_id,
            vec![replica1, replica2],
        );

        match ns.namespace_type {
            NamespaceType::MasterReplica {
                master_id: m,
                replicas,
            } => {
                assert_eq!(m, master_id);
                assert_eq!(replicas.len(), 2);
            }
            _ => panic!("wrong namespace type"),
        }
    }

    #[test]
    fn test_distributed_namespace() {
        let member1 = Uuid::new_v4();
        let member2 = Uuid::new_v4();

        let ns = Namespace::distributed(
            "cluster".to_string(),
            test_key(),
            "my-cluster".to_string(),
            vec![member1, member2],
        );

        match ns.namespace_type {
            NamespaceType::Distributed { cluster_id, members } => {
                assert_eq!(cluster_id, "my-cluster");
                assert_eq!(members.len(), 2);
            }
            _ => panic!("wrong namespace type"),
        }
    }
}
