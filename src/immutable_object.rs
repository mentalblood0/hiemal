use std::{collections::BTreeMap, ops::Bound, sync::Arc};

use parking_lot::{RwLock, RwLockReadGuard};

#[derive(Default, Clone)]
struct LockableInternals<K, V> {
    base_immutable_common_difference_option: Option<Arc<LockableInternals<K, V>>>,
    difference: BTreeMap<Arc<K>, Option<Arc<V>>>,
}

impl<'a, K, V> LockableInternals<K, V>
where
    K: Default + Ord,
    V: Default,
{
    fn insert(&mut self, key: Arc<K>, value: Arc<V>) {
        self.difference.insert(key, Some(value));
    }

    fn remove(&mut self, key: &Arc<K>) {
        if self.base_immutable_common_difference_option.is_some() {
            self.difference.insert(key.clone(), None);
        } else {
            self.difference.remove(key);
        }
    }

    fn get(&self, key: &Arc<K>) -> Option<&Arc<V>> {
        if let Some(result) = self.difference.get(key) {
            result.as_ref()
        } else {
            self.base_immutable_common_difference_option
                .as_ref()
                .and_then(|base_immutable_common_difference| {
                    base_immutable_common_difference.get(key)
                })
        }
    }

    fn iter(&'a self) -> LockableInternalsIterator<'a, K, V> {
        let mut differences_iterators = Vec::new();
        let mut current_lockable_internals_option = Some(self);
        while let Some(current_lockable_internals) = current_lockable_internals_option {
            differences_iterators.push(current_lockable_internals.difference.iter());
            current_lockable_internals_option = current_lockable_internals
                .base_immutable_common_difference_option
                .as_deref();
        }
        let current_key_value_pairs = differences_iterators
            .iter_mut()
            .map(|difference_iterator| difference_iterator.next())
            .collect::<Vec<_>>();
        LockableInternalsIterator {
            differences_iterators,
            current_key_value_pairs,
        }
    }
}

struct LockableInternalsIterator<'a, K, V> {
    differences_iterators: Vec<std::collections::btree_map::Iter<'a, Arc<K>, Option<Arc<V>>>>,
    current_key_value_pairs: Vec<Option<(&'a Arc<K>, &'a Option<Arc<V>>)>>,
}

// impl<'a, K, V> Iterator for LockableInternalsIterator<'a, K, V> {
//     type Item = (&'a Arc<K>, &'a Arc<V>);

//     fn next(&mut self) -> Option<Self::Item> {
//         for (differences_iterator, current_key_value_pair) in self
//             .differences_iterators
//             .iter_mut()
//             .zip(self.current_key_value_pairs)
//         {}
//     }
// }

#[derive(Default)]
pub struct ImmutableObject<K, V> {
    lockable_internals: RwLock<LockableInternals<K, V>>,
}

impl<K, V> Clone for ImmutableObject<K, V>
where
    K: Default + Ord + Clone,
    V: Default + Clone,
{
    fn clone(&self) -> Self {
        let mut lockable_internals_read_guard = self.lockable_internals.upgradable_read();
        if lockable_internals_read_guard.difference.len() < 16 {
            Self {
                lockable_internals: RwLock::new(LockableInternals {
                    base_immutable_common_difference_option: lockable_internals_read_guard
                        .base_immutable_common_difference_option
                        .clone(),
                    difference: lockable_internals_read_guard.difference.clone(),
                }),
            }
        } else {
            let common_base = Arc::new(lockable_internals_read_guard.clone());
            lockable_internals_read_guard.with_upgraded(|lockable_internals_write_guard| {
                lockable_internals_write_guard.base_immutable_common_difference_option =
                    Some(common_base.clone());
                lockable_internals_write_guard.difference.clear();
            });
            Self {
                lockable_internals: RwLock::new(LockableInternals {
                    base_immutable_common_difference_option: Some(common_base),
                    difference: BTreeMap::new(),
                }),
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
        self.lockable_internals.write().insert(key, value)
    }

    pub fn remove(&mut self, key: &Arc<K>) {
        self.lockable_internals.write().remove(key)
    }

    pub fn get(&self, key: &Arc<K>) -> Option<Arc<V>> {
        self.lockable_internals.write().get(key).cloned()
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
            // for object_index in 0..normal_objects.len() {
            //     assert_eq!(
            //         immutable_objects[object_index].iter().collect::<Vec<_>>(),
            //         normal_objects[object_index]
            //             .iter()
            //             .map(|(key, value)| (key.clone(), value.clone()))
            //             .collect::<Vec<_>>(),
            //         "objects at index {}",
            //         object_index
            //     );
            // }
        }
    }
}
