//! Core types for distributed tgcryptfs

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Namespace types define how data is shared across machines
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum NamespaceType {
    /// Private to this machine only
    Standalone,

    /// Shared with master-replica model
    MasterReplica {
        /// UUID of the master machine (only master can write)
        master_id: Uuid,
        /// UUIDs of replica machines (read-only access)
        replicas: Vec<Uuid>,
    },

    /// Shared with CRDT consensus (all nodes can read/write)
    Distributed {
        /// Cluster identifier
        cluster_id: String,
        /// UUIDs of all cluster members
        members: Vec<Uuid>,
    },
}

impl NamespaceType {
    /// Check if this namespace is standalone
    pub fn is_standalone(&self) -> bool {
        matches!(self, NamespaceType::Standalone)
    }

    /// Check if this namespace uses master-replica mode
    pub fn is_master_replica(&self) -> bool {
        matches!(self, NamespaceType::MasterReplica { .. })
    }

    /// Check if this namespace is fully distributed
    pub fn is_distributed(&self) -> bool {
        matches!(self, NamespaceType::Distributed { .. })
    }

    /// Get the master ID if this is a master-replica namespace
    pub fn master_id(&self) -> Option<Uuid> {
        match self {
            NamespaceType::MasterReplica { master_id, .. } => Some(*master_id),
            _ => None,
        }
    }

    /// Check if a machine ID is a member of this namespace
    pub fn is_member(&self, machine_id: Uuid) -> bool {
        match self {
            NamespaceType::Standalone => false,
            NamespaceType::MasterReplica {
                master_id,
                replicas,
            } => *master_id == machine_id || replicas.contains(&machine_id),
            NamespaceType::Distributed { members, .. } => members.contains(&machine_id),
        }
    }

    /// Check if a machine ID can write to this namespace
    pub fn can_write(&self, machine_id: Uuid) -> bool {
        match self {
            NamespaceType::Standalone => true,
            NamespaceType::MasterReplica { master_id, .. } => *master_id == machine_id,
            NamespaceType::Distributed { members, .. } => members.contains(&machine_id),
        }
    }
}

/// Access subject identifies who an access rule applies to
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub enum AccessSubject {
    /// Specific machine by UUID
    Machine(Uuid),
    /// Named group of machines
    MachineGroup(String),
    /// Any authenticated machine in the cluster
    AnyAuthenticated,
    /// Public access (no authentication required)
    Public,
}

impl AccessSubject {
    /// Check if this subject matches a given machine ID
    pub fn matches(&self, machine_id: Uuid, groups: &[String]) -> bool {
        match self {
            AccessSubject::Machine(id) => *id == machine_id,
            AccessSubject::MachineGroup(group) => groups.contains(group),
            AccessSubject::AnyAuthenticated => true,
            AccessSubject::Public => true,
        }
    }
}

/// Permissions define what operations are allowed
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Permissions {
    /// Can read files and list directories
    pub read: bool,
    /// Can create and modify files
    pub write: bool,
    /// Can delete files and directories
    pub delete: bool,
    /// Can modify ACLs and namespace settings
    pub admin: bool,
}

impl Permissions {
    /// Create permissions with all access denied
    pub fn none() -> Self {
        Self {
            read: false,
            write: false,
            delete: false,
            admin: false,
        }
    }

    /// Create read-only permissions
    pub fn read_only() -> Self {
        Self {
            read: true,
            write: false,
            delete: false,
            admin: false,
        }
    }

    /// Create read-write permissions (no delete or admin)
    pub fn read_write() -> Self {
        Self {
            read: true,
            write: true,
            delete: false,
            admin: false,
        }
    }

    /// Create full permissions (all operations allowed)
    pub fn full() -> Self {
        Self {
            read: true,
            write: true,
            delete: true,
            admin: true,
        }
    }

    /// Merge two permission sets (take the union of allowed operations)
    pub fn merge(&self, other: &Permissions) -> Permissions {
        Permissions {
            read: self.read || other.read,
            write: self.write || other.write,
            delete: self.delete || other.delete,
            admin: self.admin || other.admin,
        }
    }

    /// Check if any permission is granted
    pub fn has_any(&self) -> bool {
        self.read || self.write || self.delete || self.admin
    }
}

impl Default for Permissions {
    fn default() -> Self {
        Self::none()
    }
}

/// Access rule defines permissions for a subject on a path pattern
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessRule {
    /// Who this rule applies to
    pub subject: AccessSubject,

    /// What access is granted
    pub permissions: Permissions,

    /// Path pattern (glob-style, e.g., "/home/*", "*.txt")
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

    /// Check if this rule applies to a given path
    pub fn matches_path(&self, path: &str) -> bool {
        // Simple wildcard matching
        // TODO: Implement proper glob pattern matching
        if self.path_pattern == "*" {
            return true;
        }

        if self.path_pattern.ends_with('*') {
            let prefix = &self.path_pattern[..self.path_pattern.len() - 1];
            return path.starts_with(prefix);
        }

        path == self.path_pattern
    }

    /// Check if this rule applies to a subject and path
    pub fn applies_to(&self, machine_id: Uuid, groups: &[String], path: &str) -> bool {
        self.subject.matches(machine_id, groups) && self.matches_path(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_namespace_type_standalone() {
        let ns = NamespaceType::Standalone;
        assert!(ns.is_standalone());
        assert!(!ns.is_master_replica());
        assert!(!ns.is_distributed());
        assert!(ns.can_write(Uuid::new_v4()));
    }

    #[test]
    fn test_namespace_type_master_replica() {
        let master = Uuid::new_v4();
        let replica = Uuid::new_v4();
        let ns = NamespaceType::MasterReplica {
            master_id: master,
            replicas: vec![replica],
        };

        assert!(!ns.is_standalone());
        assert!(ns.is_master_replica());
        assert!(!ns.is_distributed());
        assert_eq!(ns.master_id(), Some(master));
        assert!(ns.can_write(master));
        assert!(!ns.can_write(replica));
        assert!(ns.is_member(master));
        assert!(ns.is_member(replica));
    }

    #[test]
    fn test_namespace_type_distributed() {
        let node1 = Uuid::new_v4();
        let node2 = Uuid::new_v4();
        let ns = NamespaceType::Distributed {
            cluster_id: "test-cluster".to_string(),
            members: vec![node1, node2],
        };

        assert!(!ns.is_standalone());
        assert!(!ns.is_master_replica());
        assert!(ns.is_distributed());
        assert!(ns.can_write(node1));
        assert!(ns.can_write(node2));
        assert!(ns.is_member(node1));
        assert!(ns.is_member(node2));
        assert!(!ns.is_member(Uuid::new_v4()));
    }

    #[test]
    fn test_permissions() {
        let none = Permissions::none();
        assert!(!none.has_any());

        let read_only = Permissions::read_only();
        assert!(read_only.read);
        assert!(!read_only.write);

        let read_write = Permissions::read_write();
        assert!(read_write.read);
        assert!(read_write.write);
        assert!(!read_write.delete);

        let full = Permissions::full();
        assert!(full.read);
        assert!(full.write);
        assert!(full.delete);
        assert!(full.admin);

        let merged = none.merge(&read_only);
        assert!(merged.read);
        assert!(!merged.write);
    }

    #[test]
    fn test_access_subject() {
        let machine_id = Uuid::new_v4();
        let other_id = Uuid::new_v4();

        let subject = AccessSubject::Machine(machine_id);
        assert!(subject.matches(machine_id, &[]));
        assert!(!subject.matches(other_id, &[]));

        let group_subject = AccessSubject::MachineGroup("admins".to_string());
        assert!(group_subject.matches(machine_id, &["admins".to_string()]));
        assert!(!group_subject.matches(machine_id, &["users".to_string()]));

        let any = AccessSubject::AnyAuthenticated;
        assert!(any.matches(machine_id, &[]));
        assert!(any.matches(other_id, &[]));
    }

    #[test]
    fn test_access_rule() {
        let machine_id = Uuid::new_v4();
        let perms = Permissions::read_only();

        let rule = AccessRule::new(
            AccessSubject::Machine(machine_id),
            perms,
            "/home/*".to_string(),
        );

        assert!(rule.matches_path("/home/user/file.txt"));
        assert!(!rule.matches_path("/etc/config"));
        assert!(rule.applies_to(machine_id, &[], "/home/user/file.txt"));
        assert!(!rule.applies_to(Uuid::new_v4(), &[], "/home/user/file.txt"));
    }

    #[test]
    fn test_access_rule_wildcard() {
        let rule = AccessRule::new(
            AccessSubject::Public,
            Permissions::read_only(),
            "*".to_string(),
        );

        assert!(rule.matches_path("/any/path"));
        assert!(rule.matches_path("file.txt"));
    }
}
