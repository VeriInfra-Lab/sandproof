//! Isolated callback-body split; the original Store and external state stay in place.
use sandbox_controller_core::Store;

pub fn toggle_step(state: &mut bool, _key: &u64, _value: &u64) -> bool {
    *state = !*state;
    *state
}

pub fn trace_step(visited: &mut Vec<(u64, u64)>, key: &u64, value: &u64, floor: u64) -> bool {
    visited.push((*key, *value));
    *value > floor
}

pub fn retain_toggle(store: &mut Store<u64, u64>, state: &mut bool) {
    store.retain(|key, value| toggle_step(state, key, value));
}

pub fn retain_trace(store: &mut Store<u64, u64>, visited: &mut Vec<(u64, u64)>, floor: u64) {
    store.retain(|key, value| trace_step(visited, key, value, floor));
}
