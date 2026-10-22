use std::{borrow::Borrow, collections::BTreeMap, hash::Hash, hash::Hasher, sync::Arc};

use parking_lot::{RwLock, RwLockReadGuard};
use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{MapAccess, Visitor},
    ser::SerializeMap,
};

use crate::merging_iterator::MergingIterator;

#[derive(Default, Clone, Debug)]
struct LockableInternals<K, V> {
    base_immutable_common_difference_option: Option<Arc<LockableInternals<K, V>>>,
    difference: BTreeMap<K, Option<V>>,
}

impl<K, V> PartialEq for LockableInternals<K, V>
where
    K: Ord + Eq,
    V: Ord + Eq,
{
    fn eq(&self, other: &Self) -> bool {
        self.iter().eq(other.iter())
    }
}

impl<K, V> Eq for LockableInternals<K, V>
where
    K: Ord + Eq,
    V: Ord + Eq,
{
}

impl<K, V> PartialOrd for LockableInternals<K, V>
where
    K: Ord + Eq,
    V: Ord + Eq,
{
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<K, V> Ord for LockableInternals<K, V>
where
    K: Ord + Eq,
    V: Ord + Eq,
{
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.iter().cmp(other.iter())
    }
}

impl<K, V> Hash for LockableInternals<K, V>
where
    K: Ord + Hash + Eq,
    V: Ord + Hash + Eq,
{
    fn hash<H: Hasher>(&self, state: &mut H) {
        for element in self.iter() {
            element.hash(state)
        }
    }
}

impl<K, V> LockableInternals<K, V>
where
    K: Ord + Clone,
{
    fn remove(&mut self, key: &K) {
        if self.base_immutable_common_difference_option.is_some() {
            self.difference.insert(key.clone(), None);
        } else {
            self.difference.remove(key);
        }
    }
}

impl<K, V> LockableInternals<K, V>
where
    K: Ord,
{
    fn insert(&mut self, key: K, value: V) {
        self.difference.insert(key, Some(value));
    }

    pub fn get<Q>(&self, key: &Q) -> Option<&V>
    where
        K: Borrow<Q>,
        Q: Ord + Eq + ?Sized,
    {
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

    fn iter<'b>(&'b self) -> Box<dyn Iterator<Item = (&'b K, &'b V)> + 'b> {
        if let Some(base_immutable_common_difference) =
            self.base_immutable_common_difference_option.as_ref()
        {
            Box::new(MergingIterator::new(
                Box::new(self.difference.iter()),
                base_immutable_common_difference.iter(),
            ))
        } else {
            Box::new(
                self.difference.iter().filter_map(|(key, value_option)| {
                    value_option.as_ref().map(|value| (key, value))
                }),
            )
        }
    }

    pub fn len(&self) -> usize {
        self.iter().count()
    }
}

#[derive(Default, Debug)]
pub struct ImmutableObject<K, V> {
    lockable_internals: RwLock<LockableInternals<K, V>>,
}

impl<K, V> Serialize for ImmutableObject<K, V>
where
    K: Ord + Serialize,
    V: Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(None)?;
        {
            let lockable_internals_read_guard = self.lockable_internals.read();
            for (k, v) in lockable_internals_read_guard.iter() {
                map.serialize_entry(k, v)?;
            }
        }
        map.end()
    }
}

impl<'de, K, V> Deserialize<'de> for ImmutableObject<K, V>
where
    K: Ord + Deserialize<'de>,
    V: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct DifferenceVisitor<K, V>(std::marker::PhantomData<(K, V)>);

        impl<'de, K, V> Visitor<'de> for DifferenceVisitor<K, V>
        where
            K: Ord + Deserialize<'de>,
            V: Deserialize<'de>,
        {
            type Value = BTreeMap<K, Option<V>>;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a map")
            }

            fn visit_map<A>(self, mut access: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut map = BTreeMap::new();
                while let Some((k, v)) = access.next_entry::<K, V>()? {
                    map.insert(k, Some(v));
                }
                Ok(map)
            }
        }

        let difference =
            deserializer.deserialize_map(DifferenceVisitor::<K, V>(std::marker::PhantomData))?;

        Ok(ImmutableObject {
            lockable_internals: RwLock::new(LockableInternals {
                base_immutable_common_difference_option: None,
                difference,
            }),
        })
    }
}

impl<K, V> PartialEq for ImmutableObject<K, V>
where
    K: Ord,
    V: Ord,
{
    fn eq(&self, other: &Self) -> bool {
        *self.lockable_internals.read() == *other.lockable_internals.read()
    }
}

impl<K, V> Eq for ImmutableObject<K, V>
where
    K: Ord,
    V: Ord,
{
}

impl<K, V> PartialOrd for ImmutableObject<K, V>
where
    K: Ord,
    V: Ord,
{
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<K, V> Ord for ImmutableObject<K, V>
where
    K: Ord,
    V: Ord,
{
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (*self.lockable_internals.read()).cmp(&*other.lockable_internals.read())
    }
}

impl<K, V> Hash for ImmutableObject<K, V>
where
    K: Ord + Hash,
    V: Ord + Hash,
{
    fn hash<H: Hasher>(&self, state: &mut H) {
        (*self.lockable_internals.read()).hash(state);
    }
}

impl<K, V> Clone for ImmutableObject<K, V>
where
    K: Clone,
    V: Clone,
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

pub struct ImmutableObjectIterator<'a, K, V> {
    lockable_internals_read_guard: RwLockReadGuard<'a, LockableInternals<K, V>>,
    lockable_internals_iterator_option: Option<Box<dyn Iterator<Item = (&'a K, &'a V)> + 'a>>,
}

impl<'a, K, V> Iterator for ImmutableObjectIterator<'a, K, V>
where
    K: Ord,
{
    type Item = (&'a K, &'a V);

    fn next(&mut self) -> Option<Self::Item> {
        if self.lockable_internals_iterator_option.is_none() {
            // SAFETY: `lockable_internals_read_guard` is stored in `self` for `'a`
            // and holds the read lock, so the `LockableInternals` it points to (and
            // its `BTreeMap`s) stay alive and unmodified for `'a`. The iterator only
            // borrows that data, so the extended lifetime is valid.
            let lockable_internals_as_const: &'a LockableInternals<K, V> = unsafe {
                &*(&*self.lockable_internals_read_guard as *const LockableInternals<K, V>)
            };
            self.lockable_internals_iterator_option = Some(lockable_internals_as_const.iter());
        }
        self.lockable_internals_iterator_option
            .as_mut()
            .unwrap()
            .next()
    }
}

impl<K, V> FromIterator<(K, V)> for ImmutableObject<K, V>
where
    K: Ord,
{
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        Self {
            lockable_internals: RwLock::new(LockableInternals {
                base_immutable_common_difference_option: None,
                difference: iter.into_iter().map(|(k, v)| (k, Some(v))).collect(),
            }),
        }
    }
}

impl<K, V> ImmutableObject<K, V>
where
    K: Ord,
{
    pub fn insert(&mut self, key: K, value: V) {
        self.lockable_internals.get_mut().insert(key, value)
    }

    pub fn get<Q>(&self, key: &Q) -> Option<V>
    where
        K: Borrow<Q>,
        Q: Ord + Eq + ?Sized,
        V: Clone,
    {
        self.lockable_internals.read().get(key).cloned()
    }

    pub fn remove(&mut self, key: &K)
    where
        K: Clone,
    {
        self.lockable_internals.get_mut().remove(key)
    }

    pub fn iter<'a>(&'a self) -> ImmutableObjectIterator<'a, K, V> {
        ImmutableObjectIterator {
            lockable_internals_read_guard: self.lockable_internals.read(),
            lockable_internals_iterator_option: None,
        }
    }

    pub fn len(&self) -> usize {
        self.lockable_internals.read().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<K, V> ImmutableObject<K, V>
where
    K: Ord + Eq,
{
    pub fn keys(&self) -> impl Iterator<Item = &K> {
        self.iter().map(|element| element.0)
    }

    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.iter().map(|element| element.1)
    }

    pub fn extend<A>(&mut self, addition: A)
    where
        A: Iterator<Item = (K, V)>,
    {
        let lockable_internals_write_guard = self.lockable_internals.get_mut();
        for key_value_pair in addition {
            lockable_internals_write_guard.insert(key_value_pair.0, key_value_pair.1);
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
        let mut normal_objects = vec![BTreeMap::<usize, usize>::new()];
        let mut immutable_objects = vec![ImmutableObject::<usize, usize>::default()];
        for _ in 0..1000 {
            let current_object_index =
                rng.generate_range(normal_objects.len().saturating_sub(20)..normal_objects.len());
            match rng.generate_range(1..=10) {
                1..=9 => {
                    let new_key = rng.generate_range(0..usize::MAX);
                    let new_value = rng.generate_range(0..usize::MAX);
                    let mut new_normal_object = normal_objects[current_object_index].clone();
                    // println!(
                    //     "insert ({new_key:?} {new_value:?}) in {current_object_index} and save as \
                    //      {}",
                    //     normal_objects.len()
                    // );
                    new_normal_object.insert(new_key, new_value);
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
                    let key_to_remove = *new_normal_object
                        .keys()
                        .nth(rng.generate_range(0..new_normal_object.len()))
                        .unwrap();
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
                    immutable_objects[object_index]
                        .lockable_internals
                        .read()
                        .iter()
                        .collect::<Vec<_>>(),
                    normal_objects[object_index].iter().collect::<Vec<_>>(),
                    "objects at index {}",
                    object_index
                );
            }
        }
    }
}
