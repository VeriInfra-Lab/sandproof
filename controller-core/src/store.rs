/// A finite association list used by the verified core.
///
/// The production journal may index rows differently, but the reducer sees a
/// finite value with last-write replacement semantics matching Lean's `put`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Store<K, V> {
    entries: Vec<(K, V)>,
}

impl<K, V> Default for Store<K, V> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
        }
    }
}

impl<K: PartialEq, V> Store<K, V> {
    #[must_use]
    pub fn get(&self, key: &K) -> Option<&V> {
        let mut index = 0;
        while index < self.entries.len() {
            if &self.entries[index].0 == key {
                return Some(&self.entries[index].1);
            }
            index += 1;
        }
        None
    }

    #[must_use]
    pub fn contains_key(&self, key: &K) -> bool {
        self.get(key).is_some()
    }

    pub fn put(&mut self, key: K, value: V) {
        let mut index = 0;
        while index < self.entries.len() {
            if self.entries[index].0 == key {
                self.entries[index].1 = value;
                return;
            }
            index += 1;
        }
        self.entries.push((key, value));
    }

    /// Remove only entries selected by the controller's checked maintenance path.
    pub fn retain(&mut self, mut keep: impl FnMut(&K, &V) -> bool) {
        self.entries.retain(move |(key, value)| keep(key, value));
    }

    #[must_use]
    pub fn entries(&self) -> &[(K, V)] {
        &self.entries
    }
}

impl<K: PartialEq, V: Copy> Store<K, V> {
    /// Return an owned copy without keeping a borrow of the store alive across
    /// later reducer branches.  All protocol ledger rows using this helper are
    /// small copyable values.
    #[must_use]
    pub fn get_copy(&self, key: &K) -> Option<V> {
        let mut index = 0;
        while index < self.entries.len() {
            if &self.entries[index].0 == key {
                return Some(self.entries[index].1);
            }
            index += 1;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::Store;

    #[test]
    fn put_replaces_without_duplicating_a_key() {
        let mut store = Store::default();
        store.put(7_u64, 11_u64);
        store.put(7_u64, 13_u64);

        assert_eq!(store.get(&7), Some(&13));
        assert_eq!(store.entries().len(), 1);
    }

    #[test]
    fn put_preserves_first_match_and_later_duplicate_rows() {
        let mut store = Store {
            entries: vec![(1_u64, 10_u64), (7, 11), (7, 12), (2, 20)],
        };
        store.put(7, 13);
        assert_eq!(store.entries(), &[(1, 10), (7, 13), (7, 12), (2, 20)]);
        assert_eq!(store.get_copy(&7), Some(13));
        assert_eq!(store.get_copy(&1), Some(10));
        assert_eq!(store.get_copy(&2), Some(20));
    }

    #[test]
    fn put_absent_key_appends_after_scanning_all_rows() {
        let mut store = Store {
            entries: vec![(1_u64, 10_u64), (7, 11), (2, 20)],
        };
        store.put(u64::MAX, u64::MAX);
        assert_eq!(
            store.entries(),
            &[(1, 10), (7, 11), (2, 20), (u64::MAX, u64::MAX)]
        );
        assert_eq!(store.get_copy(&u64::MAX), Some(u64::MAX));
        assert_eq!(store.get_copy(&7), Some(11));
    }

    #[test]
    fn retain_visits_each_original_row_once_in_order_and_preserves_survivors() {
        let mut store = Store {
            entries: vec![(1_u64, 10_u64), (7, 11), (7, 12), (2, 20)],
        };
        let mut visited = Vec::new();
        store.retain(|key, value| {
            visited.push((*key, *value));
            *value > 11
        });
        assert_eq!(visited, [(1, 10), (7, 11), (7, 12), (2, 20)]);
        assert_eq!(store.entries(), &[(7, 12), (2, 20)]);
        assert_eq!(store.get_copy(&7), Some(12));
    }

    #[test]
    fn retain_preserves_stateful_callback_results_including_empty_and_extreme_rows() {
        let mut empty = Store::<u64, u64>::default();
        empty.retain(|_, _| panic!("empty table must not invoke callback"));
        let mut store = Store {
            entries: vec![(0_u64, 0_u64), (u64::MAX, u64::MAX), (0, 9)],
        };
        let mut calls = 0;
        store.retain(|_, _| {
            calls += 1;
            calls != 2
        });
        assert_eq!(calls, 3);
        assert_eq!(store.entries(), &[(0, 0), (0, 9)]);
        store.retain(|_, _| false);
        assert!(store.entries().is_empty());
    }
}
