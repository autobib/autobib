use std::collections::{HashMap, HashSet};

use super::DatabaseFault;

/// Detects cycles in a parent-key graph structure.
///
/// Each element in `records` is a `(key, parent_key)` pair, where `parent_key = None`
/// indicates a root node. Returns a vector of cycles, where each cycle is represented
/// as a vector of keys forming the cycle.
///
/// # Example
///
/// ```
/// // Tree structure: 1 -> 2 -> 3 -> 2 (cycle between 2 and 3)
/// let records = vec![(1, None), (2, Some(1)), (3, Some(2)), (2, Some(3))];
/// let cycles = detect_cycles(&records);
/// assert!(!cycles.is_empty());
/// ```
pub fn detect_cycles(parent_map: &HashMap<i64, Option<i64>>, faults: &mut Vec<DatabaseFault>) {
    // the row-ids we have already visited
    let mut visited = HashSet::new();

    for key in parent_map.keys() {
        // Skip if we've already explored this node
        if visited.contains(key) {
            continue;
        }

        let mut path_set: HashSet<i64> = HashSet::new();
        let mut current = key;

        loop {
            // this is a cycle
            if path_set.contains(current) {
                for node in &path_set {
                    visited.insert(*node);
                }
                faults.push(DatabaseFault::ContainsCycle(path_set));
                break;
            }

            // part of an existing tree
            if visited.contains(current) {
                for node in path_set {
                    visited.insert(node);
                }
                break;
            }

            // extend the path and follow the parent
            path_set.insert(*current);

            match parent_map.get(current) {
                Some(Some(parent_key)) => {
                    current = parent_key;
                }
                Some(None) => {
                    // reached a root node; ok
                    for node in path_set {
                        visited.insert(node);
                    }
                    break;
                }
                None => {
                    // parent undefined
                    faults.push(DatabaseFault::ParentKeyMissing(*current));

                    for node in path_set {
                        visited.insert(node);
                    }
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_cycles() {
        // Simple tree: 1 <- 2 <- 3
        let mut faults = Vec::new();
        let records = [(1, None), (2, Some(1)), (3, Some(2))]
            .into_iter()
            .collect();
        detect_cycles(&records, &mut faults);
        assert!(faults.is_empty());
    }

    #[test]
    fn test_simple_cycle() {
        // Cycle: 1 -> 2 -> 1
        let mut faults = Vec::new();
        let records = [(1, Some(2)), (2, Some(1))].into_iter().collect();
        detect_cycles(&records, &mut faults);
        assert_eq!(faults.len(), 1);
    }

    #[test]
    fn test_self_loop() {
        // Self-loop: 1 -> 1
        let mut faults = Vec::new();
        let records = [(1, Some(1))].into_iter().collect();
        detect_cycles(&records, &mut faults);
        assert_eq!(faults.len(), 1);
    }

    #[test]
    fn test_multiple_trees() {
        // Two separate trees: (1 <- 2 <- 3) and (4 <- 5)
        let mut faults = Vec::new();
        let records = [
            (1, None),
            (2, Some(1)),
            (3, Some(2)),
            (4, None),
            (5, Some(4)),
        ]
        .into_iter()
        .collect();
        detect_cycles(&records, &mut faults);
        assert!(faults.is_empty());
    }

    #[test]
    fn test_multiple_cycles() {
        // Two separate cycles: (1 -> 2 -> 1) and (3 -> 4 -> 3)
        let mut faults = Vec::new();
        let records = [(1, Some(2)), (2, Some(1)), (3, Some(4)), (4, Some(3))]
            .into_iter()
            .collect();
        detect_cycles(&records, &mut faults);
        assert_eq!(faults.len(), 2);
    }
}
