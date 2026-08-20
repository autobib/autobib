use std::collections::{HashMap, HashSet};

use super::DatabaseFault;

/// Detect cycles in a parent-revision graph structure.
///
/// Each element in `parent_map` is a `(revision, parent_revision)` pair, where a missing parent
/// revision indicates a root node. Any detected faults are appended to `faults`.
///
/// # Example
///
/// ```
/// use std::collections::HashMap;
///
/// // Revision 1 is a root; revisions 2 and 3 form a cycle.
/// let records = HashMap::from([(1, None), (2, Some(3)), (3, Some(2))]);
/// let mut faults = Vec::new();
/// detect_cycles(&records, &mut faults);
/// assert!(!faults.is_empty());
/// ```
pub fn detect_cycles(parent_map: &HashMap<i64, Option<i64>>, faults: &mut Vec<DatabaseFault>) {
    // the row-ids we have already visited
    let mut visited = HashSet::new();

    for key in parent_map.keys() {
        // Skip if we've already explored this node
        if visited.contains(key) {
            continue;
        }

        let mut path = Vec::new();
        let mut path_indices = HashMap::new();
        let mut current = key;

        loop {
            // this is a cycle
            if let Some(&cycle_start) = path_indices.get(current) {
                visited.extend(path.iter().copied());
                faults.push(DatabaseFault::ContainsCycle(path[cycle_start..].to_vec()));
                break;
            }

            // part of an existing tree
            if visited.contains(current) {
                visited.extend(path);
                break;
            }

            // extend the path and follow the parent
            path_indices.insert(*current, path.len());
            path.push(*current);

            match parent_map.get(current) {
                Some(Some(parent_rev)) => {
                    current = parent_rev;
                }
                Some(None) => {
                    // reached a root node; ok
                    visited.extend(path);
                    break;
                }
                None => {
                    // parent undefined
                    faults.push(DatabaseFault::MissingParentRevision(*current));

                    visited.extend(path);
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
