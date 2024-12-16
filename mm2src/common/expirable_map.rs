//! This module provides a cross-compatible map that associates values with keys and supports expiring entries.
//!
//! Designed for performance-oriented use-cases utilizing `FxHashMap` under the hood,
//! and is not suitable for cryptographic purposes.

use instant::{Duration, Instant};
use rustc_hash::FxHashMap;
use std::{collections::BTreeMap, hash::Hash};

#[derive(Clone, Debug)]
pub struct ExpirableEntry<V> {
    pub(crate) value: V,
    pub(crate) expires_at: Instant,
}

impl<V> ExpirableEntry<V> {
    #[inline(always)]
    pub fn new(v: V, exp: Duration) -> Self {
        Self {
            expires_at: Instant::now() + exp,
            value: v,
        }
    }

    #[inline(always)]
    pub fn get_element(&self) -> &V { &self.value }

    #[inline(always)]
    pub fn update_value(&mut self, v: V) { self.value = v }

    #[inline(always)]
    pub fn update_expiration(&mut self, expires_at: Instant) { self.expires_at = expires_at }

    /// Checks whether entry has longer ttl than the given one.
    #[inline(always)]
    pub fn has_longer_life_than(&self, min_ttl: Duration) -> bool { self.expires_at > Instant::now() + min_ttl }
}

impl<K: Eq + Hash + Copy, V> Default for ExpirableMap<K, V> {
    fn default() -> Self { Self::new() }
}

/// A map that allows associating values with keys and expiring entries.
/// It is important to note that this implementation does not have a background worker to
/// automatically clear expired entries. Outdated entries are only removed when the control flow
/// is handed back to the map mutably (i.e. some mutable method of the map is invoked).
///
/// WARNING: This is designed for performance-oriented use-cases utilizing `FxHashMap`
/// under the hood and is not suitable for cryptographic purposes.
#[derive(Clone, Debug)]
pub struct ExpirableMap<K: Eq + Hash + Copy, V> {
    map: FxHashMap<K, ExpirableEntry<V>>,
    /// A sorted inverse map from expiration times to keys to speed up expired entries clearing.
    expiries: BTreeMap<Instant, K>,
}

impl<K: Eq + Hash + Copy, V> ExpirableMap<K, V> {
    /// Creates a new empty `ExpirableMap`
    #[inline]
    pub fn new() -> Self {
        Self {
            map: FxHashMap::default(),
            expiries: BTreeMap::new(),
        }
    }

    /// Returns the associated value if present and not expired.
    #[inline]
    pub fn get(&self, k: &K) -> Option<&V> {
        self.map
            .get(k)
            .filter(|v| v.expires_at > Instant::now())
            .map(|v| &v.value)
    }

    /// Removes a key-value pair from the map and returns the associated value if present and not expired.
    #[inline]
    pub fn remove(&mut self, k: &K) -> Option<V> {
        self.map.remove(k).filter(|v| v.expires_at > Instant::now()).map(|v| {
            self.expiries.remove(&v.expires_at);
            v.value
        })
    }

    /// Inserts a key-value pair with an expiration duration.
    ///
    /// If a value already exists for the given key, it will be updated and then
    /// the old one will be returned.
    pub fn insert(&mut self, k: K, v: V, exp: Duration) -> Option<V> {
        self.clear_expired_entries();
        let entry = ExpirableEntry::new(v, exp);
        self.expiries.insert(entry.expires_at, k);
        self.map.insert(k, entry).map(|v| v.value)
    }

    /// Clears the map.
    pub fn clear(&mut self) {
        self.map.clear();
        self.expiries.clear();
    }

    /// Removes expired entries from the map.
    ///
    /// Iterates through the `expiries` in order, removing entries that have expired.
    /// Stops at the first non-expired entry, leveraging the sorted nature of `BTreeMap`.
    fn clear_expired_entries(&mut self) {
        let now = Instant::now();

        // `pop_first()` is used here as it efficiently removes expired entries.
        // `first_key_value()` was considered as it wouldn't need re-insertion for
        // non-expired entries, but it would require an extra remove operation for
        // each expired entry. `pop_first()` needs only one re-insertion per call,
        // which is an acceptable trade-off compared to multiple remove operations.
        while let Some((exp, key)) = self.expiries.pop_first() {
            if exp > now {
                self.expiries.insert(exp, key);
                break;
            }
            self.map.remove(&key);
        }
    }
}

#[cfg(any(test, target_arch = "wasm32"))]
mod tests {
    use super::*;
    use crate::cross_test;
    use crate::executor::Timer;

    crate::cfg_wasm32! {
        use wasm_bindgen_test::*;
        wasm_bindgen_test::wasm_bindgen_test_configure!(run_in_browser);
    }

    cross_test!(test_clear_expired_entries, {
        let mut expirable_map = ExpirableMap::new();
        let value = "test_value";
        let exp = Duration::from_secs(1);

        // Insert 2 entries with 1 sec expiration time
        expirable_map.insert("key1", value, exp);
        expirable_map.insert("key2", value, exp);

        // Wait for entries to expire
        Timer::sleep(2.).await;

        // Clear expired entries
        expirable_map.clear_expired_entries();

        // We waited for 2 seconds, so we shouldn't have any entry accessible
        assert_eq!(expirable_map.map.len(), 0);

        // Insert 5 entries
        expirable_map.insert("key1", value, Duration::from_secs(5));
        expirable_map.insert("key2", value, Duration::from_secs(4));
        expirable_map.insert("key3", value, Duration::from_secs(7));
        expirable_map.insert("key4", value, Duration::from_secs(2));
        expirable_map.insert("key5", value, Duration::from_millis(3750));

        // Wait 2 seconds to expire some entries
        Timer::sleep(2.).await;

        // Clear expired entries
        expirable_map.clear_expired_entries();

        // We waited for 2 seconds, only one entry should expire
        assert_eq!(expirable_map.map.len(), 4);
    });
}
