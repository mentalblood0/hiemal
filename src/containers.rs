use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, hash::Hash, sync::Arc};

use crate::immutable_object::ImmutableObject;

#[derive(Clone, PartialEq, PartialOrd, Eq, Ord, Default, Serialize, Deserialize, Debug, Hash)]
#[serde(transparent)]
pub struct Object<K, V>
where
    K: Hash + Ord,
    V: Ord,
{
    pub inner: ImmutableObject<K, V>,
}

impl<K, V> Object<K, V>
where
    K: Hash + Ord,
    V: Ord,
{
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn new_from_iter(iterator: impl Iterator<Item = (Arc<K>, Arc<V>)>) -> Self {
        Self {
            inner: ImmutableObject::from_iter(iterator),
        }
    }

    pub fn insert(&mut self, key: Arc<K>, value: Arc<V>) {
        self.inner.insert(key, value);
    }

    pub fn get(&self, key: &K) -> Option<Arc<V>> {
        self.inner.get(key)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&Arc<K>, &Arc<V>)> {
        self.inner.iter()
    }

    pub fn keys(&self) -> impl Iterator<Item = &Arc<K>> {
        self.inner.keys()
    }

    pub fn values(&self) -> impl Iterator<Item = &Arc<V>> {
        self.inner.values()
    }

    pub fn extend<A>(&mut self, addition: A)
    where
        A: Iterator<Item = (Arc<K>, Arc<V>)>,
    {
        self.inner.extend(addition);
    }
}

#[derive(Clone, PartialEq, PartialOrd, Eq, Ord, Default, Serialize, Deserialize, Debug, Hash)]
#[serde(transparent)]
pub struct Set<V>
where
    V: Ord + Hash,
{
    inner: ImmutableObject<V, ()>,
}

impl<V> Set<V>
where
    V: Ord + Clone + Hash,
{
    pub fn contains(&self, value: &V) -> bool {
        self.inner.contains(value)
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn insert(&mut self, value: Arc<V>) {
        self.inner.insert(value, ().into());
    }

    pub fn extend<A>(&mut self, addition: A)
    where
        A: Iterator<Item = Arc<V>>,
    {
        self.inner.extend(addition.map(|key| (key, ().into())));
    }
}

#[derive(Clone, PartialEq, PartialOrd, Eq, Ord, Debug, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Vector<V>
where
    V: Ord + Clone + 'static,
{
    pub inner: Vec<Arc<V>>,
}

impl<V> Vector<V>
where
    V: Ord + Clone,
{
    pub fn new_from_iter(iterator: impl Iterator<Item = Arc<V>>) -> Self {
        Self {
            inner: Vec::from_iter(iterator),
        }
    }

    pub fn set(&mut self, index: usize, value: Arc<V>) {
        self.inner[index] = value
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn append(&mut self, iterator: impl Iterator<Item = Arc<V>>) {
        for new_element in iterator {
            self.inner.push(new_element);
        }
    }

    pub fn get(&self, index: usize) -> Option<&Arc<V>> {
        self.inner.get(index)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn iter(&self) -> impl Iterator<Item = &Arc<V>> {
        self.inner.iter()
    }

    pub fn push(&mut self, value: Arc<V>) {
        self.inner.push(value);
    }

    pub fn extend<A>(&mut self, addition: A)
    where
        A: IntoIterator<Item = Arc<V>>,
    {
        for value in addition {
            self.inner.push(value);
        }
    }

    pub fn extended<A>(&self, addition: A) -> Self
    where
        A: IntoIterator<Item = Arc<V>>,
    {
        let mut result = self.clone();
        result.extend(addition);
        result
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            inner: Vec::with_capacity(capacity),
        }
    }
}

impl<V> Default for Vector<V>
where
    V: Ord + Clone + 'static,
{
    fn default() -> Self {
        Self { inner: Vec::new() }
    }
}
