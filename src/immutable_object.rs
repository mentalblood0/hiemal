use std::{collections::BTreeMap, ops::Bound, sync::Arc};

use parking_lot::{RwLock, RwLockReadGuard};

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
        self.difference.insert(key.clone(), None);
    }

    fn get(&self, key: &Arc<K>) -> Option<Arc<V>> {
        if let Some(result) = self.difference.get(key) {
            result.clone()
        } else if let Some(ref base_version_lockable_internals) =
            self.base_version_lockable_internals_option
        {
            base_version_lockable_internals.read().get(key).clone()
        } else {
            None
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
        let lockable_internals_read_guard = lockable_internals_outer_read_guard.read();
        if lockable_internals_read_guard.difference.len() < 16 {
            Self {
                lockable_internals: RwLock::new(Arc::new(RwLock::new(LockableInternals {
                    base_version_lockable_internals_option: lockable_internals_read_guard
                        .base_version_lockable_internals_option
                        .clone(),
                    difference: lockable_internals_read_guard.difference.clone(),
                }))),
            }
        } else {
            drop(lockable_internals_read_guard);
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
                lockable_internals: RwLock::new(Arc::new(RwLock::new(LockableInternals {
                    base_version_lockable_internals_option: Some(
                        common_base_version_lockable_internals,
                    ),
                    difference: BTreeMap::new(),
                }))),
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

    pub fn iter(&self) -> ImmutableObjectIterator<K, V> {
        let head = self.lockable_internals.read().clone();

        let mut guards = Vec::new();
        let mut cursors = Vec::new();
        let mut current_cursors_elements = Vec::new();
        let mut current_arc = Some(head.clone());

        while let Some(arc) = current_arc {
            let guard = arc.read();
            let mut cursor: Box<dyn Iterator<Item = Entry<K, V>>> =
                Box::new(guard.difference.iter().map(|(k, v)| (k.clone(), v.clone())));

            let base = guard.base_version_lockable_internals_option.clone();

            guards.push(guard);
            current_cursors_elements.push(cursor.next());
            cursors.push(cursor);
            current_arc = base;
        }

        ImmutableObjectIterator {
            _head: head,
            guards,
            cursors,
            current_cursors_elements,
        }
    }
}

type Entry<K, V> = (Arc<K>, Option<Arc<V>>);

pub struct ImmutableObjectIterator<'a, K: Ord + 'static, V: 'static> {
    _head: Arc<RwLock<LockableInternals<K, V>>>,
    guards: Vec<RwLockReadGuard<'a, LockableInternals<K, V>>>,
    cursors: Vec<Box<dyn Iterator<Item = Entry<K, V>> + 'a>>,
    current_cursors_elements: Vec<Option<Entry<K, V>>>,
}

impl<'a, K: Ord, V> Iterator for ImmutableObjectIterator<'a, K, V> {
    type Item = (Arc<K>, Arc<V>);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let mut min_key_entry = None;
            for entry in self.current_cursors_elements.iter() {
                match &min_key_entry {
                    None => min_key_entry = entry.clone(),
                    // Some(cur_min) if *key < *cur_min => min_key = Some(key.clone()),
                    Some((min_key, _)) if &entry.as_ref().unwrap().0 < min_key => {
                        min_key_entry = entry.clone()
                    }
                    _ => {}
                }
            }
            let min_key_entry = min_key_entry?;
            for (cursor, current_cursor_element) in self
                .cursors
                .iter_mut()
                .zip(self.current_cursors_elements.iter_mut())
            {
                if let Some((key, _)) = current_cursor_element
                    && key == &min_key_entry.0
                {
                    *current_cursor_element = cursor.next();
                }
            }
            if let Some(result_value) = min_key_entry.1 {
                return Some((min_key_entry.0, result_value));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::BTreeMap;

    use nanorand::{Rng, WyRand};
    use pretty_assertions::assert_eq;

    #[test]
    fn test_generative() {
        let mut rng = WyRand::new_seed(0);
        let mut normal_objects = vec![BTreeMap::<Arc<usize>, Arc<usize>>::new()];
        let mut immutable_objects = vec![ImmutableObject::<usize, usize>::default()];
        for _ in 0..1000 {
            let current_object_index =
                rng.generate_range(normal_objects.len().saturating_sub(20)..normal_objects.len());
            match rng.generate_range(1..=10) {
                1..=9 => {
                    let new_key = Arc::new(rng.generate_range(0..usize::MAX));
                    let new_value = Arc::new(rng.generate_range(0..usize::MAX));
                    let mut new_normal_object = normal_objects[current_object_index].clone();
                    // println!(
                    //     "insert ({new_key:?} {new_value:?}) in {current_object_index} and save as \
                    //      {}",
                    //     normal_objects.len()
                    // );
                    new_normal_object.insert(new_key.clone(), new_value.clone());
                    normal_objects.push(new_normal_object);
                    let mut new_immutable_object = immutable_objects[current_object_index].clone();
                    new_immutable_object.insert(new_key, new_value);
                    immutable_objects.push(new_immutable_object);
                }
                10 => {
                    let mut new_normal_object = normal_objects[current_object_index].clone();
                    if new_normal_object.is_empty() {
                        continue;
                    }
                    let key_to_remove = new_normal_object
                        .keys()
                        .nth(rng.generate_range(0..new_normal_object.len()))
                        .unwrap()
                        .clone();
                    // println!(
                    //     "remove {key_to_remove:?} from {current_object_index} and save as {}",
                    //     normal_objects.len()
                    // );
                    new_normal_object.remove(&key_to_remove);
                    normal_objects.push(new_normal_object);
                    let mut new_immutable_object = immutable_objects[current_object_index].clone();
                    new_immutable_object.remove(&key_to_remove);
                    immutable_objects.push(new_immutable_object);
                }
                _ => {}
            }
            for object_index in 0..normal_objects.len() {
                assert_eq!(
                    immutable_objects[object_index].iter().collect::<Vec<_>>(),
                    normal_objects[object_index]
                        .iter()
                        .map(|(key, value)| (key.clone(), value.clone()))
                        .collect::<Vec<_>>(),
                    "objects at index {}",
                    object_index
                );
            }
        }
    }
}
