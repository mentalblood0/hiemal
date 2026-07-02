use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{SeqAccess, Visitor},
    ser::SerializeSeq,
};
use std::fmt;
use std::marker::PhantomData;

#[derive(Clone, PartialEq, PartialOrd, Eq, Ord, Default, Serialize, Deserialize, Debug, Hash)]
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

#[derive(Clone, PartialEq, PartialOrd, Eq, Ord, Default, Serialize, Deserialize, Debug, Hash)]
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

#[derive(Clone, PartialEq, PartialOrd, Eq, Ord, Debug, Hash)]
pub struct List<V>
where
    V: Ord + Clone + 'static,
{
    pub inner: im_lists::list::SharedList<V>,
}

impl<V> List<V>
where
    V: Ord + Clone,
{
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = &V> {
        self.inner.iter()
    }

    pub fn push_back_mut(&mut self, value: V) {
        self.inner.push_back(value);
    }

    pub fn append_mut(&mut self, other: Self) {
        self.inner.append_mut(other.inner);
    }

    pub fn extend<A>(&mut self, addition: A)
    where
        A: IntoIterator<Item = V>,
    {
        for value in addition {
            self.inner.push_back(value);
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

impl<V> Default for List<V>
where
    V: Ord + Clone + 'static,
{
    fn default() -> Self {
        Self {
            inner: im_lists::list::SharedList::new(),
        }
    }
}

impl<V> Serialize for List<V>
where
    V: Ord + Clone + Serialize + 'static,
{
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut result = serializer.serialize_seq(Some(self.len()))?;
        for element in self.iter() {
            result.serialize_element(element)?;
        }
        result.end()
    }
}

impl<'de, V> Deserialize<'de> for List<V>
where
    V: Ord + Clone + Deserialize<'de> + 'static,
{
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct VectorVisitor<V> {
            marker: PhantomData<V>,
        }

        impl<'de, V: Deserialize<'de>> Visitor<'de> for VectorVisitor<V>
        where
            V: Ord + Clone + 'static,
        {
            type Value = List<V>;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a sequence")
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut result = List::default();
                while let Some(element) = seq.next_element()? {
                    result.push_back_mut(element);
                }
                Ok(result)
            }
        }

        deserializer.deserialize_seq(VectorVisitor {
            marker: PhantomData,
        })
    }
}
