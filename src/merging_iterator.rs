use std::cmp::Ordering;

pub struct MergingIterator<'a, K, V> {
    pub new_iter: Box<dyn Iterator<Item = (&'a K, &'a Option<V>)> + 'a>,
    pub old_iter: Box<dyn Iterator<Item = (&'a K, &'a V)> + 'a>,
    pub current_new_keyvalue_option: Option<(&'a K, &'a Option<V>)>,
    pub current_old_keyvalue_option: Option<(&'a K, &'a V)>,
}

impl<'a, K, V> MergingIterator<'a, K, V> {
    pub fn new(
        mut new_iter: Box<dyn Iterator<Item = (&'a K, &'a Option<V>)> + 'a>,
        mut old_iter: Box<dyn Iterator<Item = (&'a K, &'a V)> + 'a>,
    ) -> Self {
        let current_new_keyvalue_option = new_iter.next();
        let current_old_keyvalue_option = old_iter.next();
        Self {
            new_iter,
            old_iter,
            current_new_keyvalue_option,
            current_old_keyvalue_option,
        }
    }
}

impl<'a, K, V> Iterator for MergingIterator<'a, K, V>
where
    K: Ord,
{
    type Item = (&'a K, &'a V);

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            match (
                &self.current_new_keyvalue_option,
                &mut self.current_old_keyvalue_option,
            ) {
                (Some(current_new_keyvalue), Some(_)) => {
                    match current_new_keyvalue
                        .0
                        .cmp(self.current_old_keyvalue_option.as_ref().unwrap().0)
                    {
                        Ordering::Less => {
                            let result = current_new_keyvalue
                                .1
                                .as_ref()
                                .map(|value| (current_new_keyvalue.0, value));
                            self.current_new_keyvalue_option = self.new_iter.next();
                            if result.is_some() {
                                return result;
                            }
                        }
                        Ordering::Greater => {
                            if let Some(current_old_keyvalue) = self.current_old_keyvalue_option {
                                let result = current_old_keyvalue;
                                self.current_old_keyvalue_option = self.old_iter.next();
                                return Some(result);
                            } else {
                                let result = current_new_keyvalue
                                    .1
                                    .as_ref()
                                    .map(|value| (current_new_keyvalue.0, value));
                                if result.is_some() {
                                    return result;
                                }
                            }
                        }
                        Ordering::Equal => {
                            let result = current_new_keyvalue
                                .1
                                .as_ref()
                                .map(|value| (current_new_keyvalue.0, value));
                            self.current_new_keyvalue_option = self.new_iter.next();
                            self.current_old_keyvalue_option = self.old_iter.next();
                            if result.is_some() {
                                return result;
                            }
                        }
                    }
                }
                (Some(current_new_keyvalue), None) => {
                    let result = current_new_keyvalue
                        .1
                        .as_ref()
                        .map(|value| (current_new_keyvalue.0, value));
                    self.current_new_keyvalue_option = self.new_iter.next();
                    if result.is_some() {
                        return result;
                    }
                }
                (None, Some(_)) => {
                    return std::mem::replace(
                        &mut self.current_old_keyvalue_option,
                        self.old_iter.next(),
                    );
                }
                (None, None) => return None,
            }
        }
    }
}
