use std::{collections::BTreeMap, sync::Arc};

use parking_lot::RwLock;

#[derive(Default)]
struct LockableInternals<K, V> {
    base_version_lockable_internals_option: Option<Arc<RwLock<LockableInternals<K, V>>>>,
    difference: BTreeMap<Arc<K>, Option<Arc<V>>>,
}

impl<K, V> LockableInternals<K, V>
where
    K: Default + Ord,
    V: Default,
{
    fn insert(&mut self, key: Arc<K>, value: Arc<V>) {
        self.difference.insert(key, Some(value));
    }

    fn remove(&mut self, key: &Arc<K>) {
        if let Some(value) = self.difference.get_mut(key) {
            *value = None;
        } else {
            self.difference.insert(key.clone(), None);
        }
    }

    fn get(&self, key: &Arc<K>) -> Option<Arc<V>> {
        if let Some(result) = self.difference.get(key) {
            result.clone()
        } else {
            if let Some(ref base_version_lockable_internals) =
                self.base_version_lockable_internals_option
            {
                base_version_lockable_internals.read().get(key).clone()
            } else {
                None
            }
        }
    }
}

#[derive(Default)]
pub struct ImmutableObject<K, V> {
    lockable_internals: RwLock<Arc<RwLock<LockableInternals<K, V>>>>,
}

impl<K, V> Clone for ImmutableObject<K, V>
where
    K: Default + Ord,
    V: Default,
{
    fn clone(&self) -> Self {
        let mut lockable_internals_outer_read_guard = self.lockable_internals.upgradable_read();
        if lockable_internals_outer_read_guard.read().difference.len() < 16 {
            Self {
                lockable_internals: RwLock::new(lockable_internals_outer_read_guard.clone()),
            }
        } else {
            let common_base_version_lockable_internals =
                lockable_internals_outer_read_guard.clone();
            lockable_internals_outer_read_guard.with_upgraded(
                |lockable_internals_outer_write_guard| {
                    *lockable_internals_outer_write_guard = Arc::default();
                    lockable_internals_outer_write_guard
                        .write()
                        .base_version_lockable_internals_option =
                        Some(common_base_version_lockable_internals.clone());
                },
            );
            Self {
                lockable_internals: RwLock::new(common_base_version_lockable_internals),
            }
        }
    }
}

impl<K, V> ImmutableObject<K, V>
where
    K: Default + Ord,
    V: Default,
{
    pub fn insert(&mut self, key: Arc<K>, value: Arc<V>) {
        let lockable_internals_outer_read_guard = self.lockable_internals.read();
        lockable_internals_outer_read_guard
            .write()
            .insert(key, value)
    }

    pub fn remove(&mut self, key: &Arc<K>) {
        let lockable_internals_outer_read_guard = self.lockable_internals.read();
        lockable_internals_outer_read_guard.write().remove(key)
    }

    pub fn get(&self, key: &Arc<K>) -> Option<Arc<V>> {
        let lockable_internals_outer_read_guard = self.lockable_internals.read();
        lockable_internals_outer_read_guard.read().get(key)
    }
}
