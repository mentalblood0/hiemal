use serde::{Deserialize, Serialize};

#[derive(Clone, PartialEq, PartialOrd, Eq, Ord, Default, Serialize, Deserialize, Debug)]
#[serde(transparent)]
pub struct Map<K, V>
where
    K: Ord + Clone,
    V: Clone,
{
    pub inner: rpds::RedBlackTreeMapSync<K, V>,
}

impl<K, V> Map<K, V>
where
    K: Ord + Clone,
    V: Clone,
{
    pub fn extend<A>(&mut self, addition: A)
    where
        A: IntoIterator<Item = (K, V)>,
    {
        for (key, value) in addition {
            self.inner.insert_mut(key, value);
        }
    }

    pub fn extended<A>(&self, addition: A) -> Self
    where
        A: IntoIterator<Item = (K, V)>,
    {
        let mut result = self.clone();
        result.extend(addition);
        result
    }
}

#[derive(Clone, PartialEq, PartialOrd, Eq, Ord, Default, Serialize, Deserialize, Debug)]
#[serde(transparent)]
pub struct Set<V>
where
    V: Ord + Clone,
{
    pub inner: rpds::RedBlackTreeSetSync<V>,
}

impl<V> Set<V>
where
    V: Ord + Clone,
{
    pub fn extend<A>(&mut self, addition: A)
    where
        A: IntoIterator<Item = V>,
    {
        for value in addition {
            self.inner.insert_mut(value);
        }
    }

    pub fn extended<A>(&self, addition: A) -> Self
    where
        A: IntoIterator<Item = V>,
    {
        let mut result = self.clone();
        result.extend(addition);
        result
    }
}

#[derive(Clone, PartialEq, PartialOrd, Eq, Ord, Default, Serialize, Deserialize, Debug)]
#[serde(transparent)]
pub struct Vector<V>
where
    V: Ord + Clone,
{
    pub inner: rpds::VectorSync<V>,
}

impl<V> Vector<V>
where
    V: Ord + Clone,
{
    pub fn extend<A>(&mut self, addition: A)
    where
        A: IntoIterator<Item = V>,
    {
        for value in addition {
            self.inner.push_back_mut(value);
        }
    }

    pub fn extended<A>(&self, addition: A) -> Self
    where
        A: IntoIterator<Item = V>,
    {
        let mut result = self.clone();
        result.extend(addition);
        result
    }
}
