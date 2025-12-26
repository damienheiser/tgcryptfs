//! Vector clock implementation for distributed causality tracking

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Vector clock for tracking causality in distributed systems
///
/// A vector clock is a data structure used for determining the partial ordering of events
/// in a distributed system and detecting causality violations. Each machine maintains a
/// logical timestamp that is incremented on local events and merged when receiving events
/// from other machines.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VectorClock {
    /// Maps machine ID to logical timestamp
    clocks: HashMap<Uuid, u64>,
}

impl VectorClock {
    /// Create a new empty vector clock
    pub fn new() -> Self {
        Self {
            clocks: HashMap::new(),
        }
    }

    /// Create a vector clock with initial timestamp for a machine
    pub fn with_initial(machine_id: Uuid, timestamp: u64) -> Self {
        let mut clocks = HashMap::new();
        clocks.insert(machine_id, timestamp);
        Self { clocks }
    }

    /// Increment the logical timestamp for a machine
    ///
    /// This should be called when a local event occurs on the machine.
    pub fn increment(&mut self, machine_id: Uuid) {
        let entry = self.clocks.entry(machine_id).or_insert(0);
        *entry += 1;
    }

    /// Get the current timestamp for a machine
    pub fn get(&self, machine_id: Uuid) -> u64 {
        self.clocks.get(&machine_id).copied().unwrap_or(0)
    }

    /// Set the timestamp for a machine
    pub fn set(&mut self, machine_id: Uuid, timestamp: u64) {
        self.clocks.insert(machine_id, timestamp);
    }

    /// Merge another vector clock into this one
    ///
    /// For each machine, takes the maximum timestamp. This is used when
    /// receiving events from other machines to update local knowledge of
    /// the distributed state.
    pub fn merge(&mut self, other: &VectorClock) {
        for (&machine_id, &timestamp) in &other.clocks {
            let entry = self.clocks.entry(machine_id).or_insert(0);
            *entry = (*entry).max(timestamp);
        }
    }

    /// Check if this vector clock happened before another (self < other)
    ///
    /// Returns true if:
    /// - For all machines, self[m] <= other[m]
    /// - For at least one machine, self[m] < other[m]
    ///
    /// This indicates a causal relationship: self happened before other.
    pub fn happened_before(&self, other: &VectorClock) -> bool {
        let mut all_less_or_equal = true;
        let mut at_least_one_less = false;

        // Get all unique machine IDs from both clocks
        let mut all_machines: Vec<Uuid> = self.clocks.keys().copied().collect();
        for &machine_id in other.clocks.keys() {
            if !all_machines.contains(&machine_id) {
                all_machines.push(machine_id);
            }
        }

        for &machine_id in &all_machines {
            let self_time = self.get(machine_id);
            let other_time = other.get(machine_id);

            if self_time > other_time {
                all_less_or_equal = false;
                break;
            }
            if self_time < other_time {
                at_least_one_less = true;
            }
        }

        all_less_or_equal && at_least_one_less
    }

    /// Check if this vector clock happened after another (self > other)
    pub fn happened_after(&self, other: &VectorClock) -> bool {
        other.happened_before(self)
    }

    /// Check if two vector clocks are concurrent (neither happened before the other)
    ///
    /// Returns true if there exist machines m1 and m2 such that:
    /// - self[m1] > other[m1]
    /// - self[m2] < other[m2]
    ///
    /// This indicates that the events are concurrent and have no causal relationship.
    pub fn concurrent(&self, other: &VectorClock) -> bool {
        !self.happened_before(other) && !other.happened_before(self) && self != other
    }

    /// Compare two vector clocks and return their relationship
    pub fn compare(&self, other: &VectorClock) -> ClockOrdering {
        if self == other {
            ClockOrdering::Equal
        } else if self.happened_before(other) {
            ClockOrdering::Before
        } else if self.happened_after(other) {
            ClockOrdering::After
        } else {
            ClockOrdering::Concurrent
        }
    }

    /// Get all machines tracked by this vector clock
    pub fn machines(&self) -> Vec<Uuid> {
        self.clocks.keys().copied().collect()
    }

    /// Check if this vector clock is empty (no machines tracked)
    pub fn is_empty(&self) -> bool {
        self.clocks.is_empty()
    }

    /// Get the number of machines tracked
    pub fn len(&self) -> usize {
        self.clocks.len()
    }

    /// Create a new vector clock that is the result of merging two clocks
    pub fn merged(&self, other: &VectorClock) -> VectorClock {
        let mut result = self.clone();
        result.merge(other);
        result
    }

    /// Clear all timestamps
    pub fn clear(&mut self) {
        self.clocks.clear();
    }
}

impl Default for VectorClock {
    fn default() -> Self {
        Self::new()
    }
}

/// Ordering relationship between two vector clocks
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockOrdering {
    /// Clocks are equal
    Equal,
    /// First clock happened before second
    Before,
    /// First clock happened after second
    After,
    /// Clocks are concurrent (no causal relationship)
    Concurrent,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_vector_clock() {
        let vc = VectorClock::new();
        assert!(vc.is_empty());
        assert_eq!(vc.len(), 0);
    }

    #[test]
    fn test_increment() {
        let mut vc = VectorClock::new();
        let machine_id = Uuid::new_v4();

        assert_eq!(vc.get(machine_id), 0);

        vc.increment(machine_id);
        assert_eq!(vc.get(machine_id), 1);

        vc.increment(machine_id);
        assert_eq!(vc.get(machine_id), 2);
    }

    #[test]
    fn test_merge() {
        let machine_a = Uuid::new_v4();
        let machine_b = Uuid::new_v4();

        let mut vc1 = VectorClock::new();
        vc1.set(machine_a, 3);
        vc1.set(machine_b, 1);

        let mut vc2 = VectorClock::new();
        vc2.set(machine_a, 2);
        vc2.set(machine_b, 4);

        vc1.merge(&vc2);

        // Should take max of each
        assert_eq!(vc1.get(machine_a), 3);
        assert_eq!(vc1.get(machine_b), 4);
    }

    #[test]
    fn test_happened_before() {
        let machine_a = Uuid::new_v4();
        let machine_b = Uuid::new_v4();

        // vc1: {A:1, B:1}
        let mut vc1 = VectorClock::new();
        vc1.set(machine_a, 1);
        vc1.set(machine_b, 1);

        // vc2: {A:2, B:2}
        let mut vc2 = VectorClock::new();
        vc2.set(machine_a, 2);
        vc2.set(machine_b, 2);

        // vc1 happened before vc2
        assert!(vc1.happened_before(&vc2));
        assert!(!vc2.happened_before(&vc1));
    }

    #[test]
    fn test_happened_after() {
        let machine_a = Uuid::new_v4();

        let mut vc1 = VectorClock::new();
        vc1.set(machine_a, 1);

        let mut vc2 = VectorClock::new();
        vc2.set(machine_a, 2);

        assert!(!vc1.happened_after(&vc2));
        assert!(vc2.happened_after(&vc1));
    }

    #[test]
    fn test_concurrent() {
        let machine_a = Uuid::new_v4();
        let machine_b = Uuid::new_v4();

        // vc1: {A:2, B:1}
        let mut vc1 = VectorClock::new();
        vc1.set(machine_a, 2);
        vc1.set(machine_b, 1);

        // vc2: {A:1, B:2}
        let mut vc2 = VectorClock::new();
        vc2.set(machine_a, 1);
        vc2.set(machine_b, 2);

        // These are concurrent
        assert!(vc1.concurrent(&vc2));
        assert!(vc2.concurrent(&vc1));

        // Not concurrent with itself
        assert!(!vc1.concurrent(&vc1));
    }

    #[test]
    fn test_concurrent_with_different_machines() {
        let machine_a = Uuid::new_v4();
        let machine_b = Uuid::new_v4();

        // vc1 knows only about machine A
        let mut vc1 = VectorClock::new();
        vc1.set(machine_a, 5);

        // vc2 knows only about machine B
        let mut vc2 = VectorClock::new();
        vc2.set(machine_b, 3);

        // These should be concurrent
        assert!(vc1.concurrent(&vc2));
        assert!(vc2.concurrent(&vc1));
    }

    #[test]
    fn test_compare() {
        let machine_a = Uuid::new_v4();

        let mut vc1 = VectorClock::new();
        vc1.set(machine_a, 1);

        let mut vc2 = VectorClock::new();
        vc2.set(machine_a, 2);

        assert_eq!(vc1.compare(&vc1), ClockOrdering::Equal);
        assert_eq!(vc1.compare(&vc2), ClockOrdering::Before);
        assert_eq!(vc2.compare(&vc1), ClockOrdering::After);
    }

    #[test]
    fn test_compare_concurrent() {
        let machine_a = Uuid::new_v4();
        let machine_b = Uuid::new_v4();

        let mut vc1 = VectorClock::new();
        vc1.set(machine_a, 2);
        vc1.set(machine_b, 1);

        let mut vc2 = VectorClock::new();
        vc2.set(machine_a, 1);
        vc2.set(machine_b, 2);

        assert_eq!(vc1.compare(&vc2), ClockOrdering::Concurrent);
    }

    #[test]
    fn test_merged() {
        let machine_a = Uuid::new_v4();
        let machine_b = Uuid::new_v4();

        let mut vc1 = VectorClock::new();
        vc1.set(machine_a, 3);

        let mut vc2 = VectorClock::new();
        vc2.set(machine_b, 2);

        let merged = vc1.merged(&vc2);
        assert_eq!(merged.get(machine_a), 3);
        assert_eq!(merged.get(machine_b), 2);

        // Original should be unchanged
        assert_eq!(vc1.get(machine_b), 0);
    }

    #[test]
    fn test_machines() {
        let machine_a = Uuid::new_v4();
        let machine_b = Uuid::new_v4();

        let mut vc = VectorClock::new();
        vc.set(machine_a, 1);
        vc.set(machine_b, 2);

        let machines = vc.machines();
        assert_eq!(machines.len(), 2);
        assert!(machines.contains(&machine_a));
        assert!(machines.contains(&machine_b));
    }

    #[test]
    fn test_clear() {
        let machine_a = Uuid::new_v4();
        let mut vc = VectorClock::new();
        vc.set(machine_a, 5);

        assert!(!vc.is_empty());
        vc.clear();
        assert!(vc.is_empty());
        assert_eq!(vc.get(machine_a), 0);
    }

    #[test]
    fn test_serialization() {
        let machine_a = Uuid::new_v4();
        let machine_b = Uuid::new_v4();

        let mut vc = VectorClock::new();
        vc.set(machine_a, 3);
        vc.set(machine_b, 7);

        let json = serde_json::to_string(&vc).expect("Failed to serialize");
        let deserialized: VectorClock = serde_json::from_str(&json).expect("Failed to deserialize");

        assert_eq!(vc, deserialized);
        assert_eq!(deserialized.get(machine_a), 3);
        assert_eq!(deserialized.get(machine_b), 7);
    }

    #[test]
    fn test_complex_causality_scenario() {
        // Simulate a distributed scenario with 3 machines
        let machine_a = Uuid::new_v4();
        let machine_b = Uuid::new_v4();
        let machine_c = Uuid::new_v4();

        // Initial state: all at 0
        let mut vc_a = VectorClock::new();
        let mut vc_b = VectorClock::new();
        let mut vc_c = VectorClock::new();

        // Machine A performs operation 1
        vc_a.increment(machine_a); // A:{A:1}

        // Machine B performs operation 1
        vc_b.increment(machine_b); // B:{B:1}

        // These operations are concurrent
        assert!(vc_a.concurrent(&vc_b));

        // Machine A receives B's operation
        vc_a.merge(&vc_b); // A:{A:1, B:1}

        // Machine A performs operation 2
        vc_a.increment(machine_a); // A:{A:2, B:1}

        // Machine C receives A's current state
        vc_c.merge(&vc_a); // C:{A:2, B:1}

        // Machine C performs operation 1
        vc_c.increment(machine_c); // C:{A:2, B:1, C:1}

        // Now C's operation happened after A's original operation
        let mut vc_a_snapshot = VectorClock::new();
        vc_a_snapshot.set(machine_a, 1);
        assert!(vc_a_snapshot.happened_before(&vc_c));

        // B's original operation happened before C's operation
        // (C received B's state via A's merge, so C knows about B:1)
        assert!(vc_b.happened_before(&vc_c));
    }

    #[test]
    fn test_equal_clocks_not_concurrent() {
        let machine_a = Uuid::new_v4();
        let mut vc = VectorClock::new();
        vc.set(machine_a, 5);

        // A clock is not concurrent with itself
        assert!(!vc.concurrent(&vc));
        assert_eq!(vc.compare(&vc), ClockOrdering::Equal);
    }

    #[test]
    fn test_partial_overlap() {
        let machine_a = Uuid::new_v4();
        let machine_b = Uuid::new_v4();
        let machine_c = Uuid::new_v4();

        // vc1: {A:3, B:2}
        let mut vc1 = VectorClock::new();
        vc1.set(machine_a, 3);
        vc1.set(machine_b, 2);

        // vc2: {B:1, C:5}
        let mut vc2 = VectorClock::new();
        vc2.set(machine_b, 1);
        vc2.set(machine_c, 5);

        // These are concurrent (vc1[A] > vc2[A]=0, but vc1[C]=0 < vc2[C])
        assert!(vc1.concurrent(&vc2));
    }
}
