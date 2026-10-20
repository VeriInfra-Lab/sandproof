//! Shared-borrow roots call the actual finite store; no copied search algorithm.
use sandbox_controller_core::Store;

#[must_use]
pub fn lookup<'a, K: PartialEq, V>(store: &'a Store<K, V>, key: &K) -> Option<&'a V> {
    store.get(key)
}

#[must_use]
pub fn contains<K: PartialEq, V>(store: &Store<K, V>, key: &K) -> bool {
    store.contains_key(key)
}

#[must_use]
pub fn lookup_u64<'a>(store: &'a Store<u64, u64>, key: &u64) -> Option<&'a u64> {
    lookup(store, key)
}

#[must_use]
pub fn contains_u64(store: &Store<u64, u64>, key: &u64) -> bool {
    contains(store, key)
}
