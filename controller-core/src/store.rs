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

    #[cfg(test)]
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
}
