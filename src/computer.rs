use std::{borrow::Cow, hash::Hash, hash::Hasher, io::Read, sync::Arc};

use anyhow::{Context, Result, anyhow};
use dashu::Rational;
use gxhash::HashMap;
use parking_lot::{Mutex, RwLock};
use rayon::prelude::*;
use rpds::RedBlackTreeMapSync;
use serde::{Deserialize, Serialize};

use crate::intermediate_representation::{self, RangeBound, Throughs};
use crate::{
    containers::{List, Map},
    intermediate_representation::{
        Condition, Content, EmbeddedFunction, IntermediateRepresentation, Node, ValuePathSegment,
    },
    value::Value,
};

#[derive(Clone, Debug)]
struct LazyValue {
    node: Arc<Node>,
    constants: rpds::VectorSync<Option<IntermediateValue>>,
}

#[derive(Clone, Debug)]
struct SequenceLockableInternals {
    next_lazy_value: Option<LazyValue>,
    already_computed_values: List<IntermediateValue>,
}

#[derive(Clone, Debug)]
struct Sequence {
    intermediate_representation_content: Arc<intermediate_representation::Sequence>,
    lockable_internals: Arc<RwLock<SequenceLockableInternals>>,
}

impl PartialEq for Sequence {
    fn eq(&self, other: &Self) -> bool {
        self.intermediate_representation_content == other.intermediate_representation_content
    }
}

impl Eq for Sequence {}

impl PartialOrd for Sequence {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Sequence {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.intermediate_representation_content
            .cmp(&other.intermediate_representation_content)
    }
}

impl Hash for Sequence {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.intermediate_representation_content.hash(state);
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
enum IntermediateValue {
    Value(Option<Value>),
    Tuple(List<IntermediateValue>),
    Object(Map<String, IntermediateValue>),
    Sequence(Sequence),
}

impl Default for IntermediateValue {
    fn default() -> Self {
        Self::Value(None)
    }
}

#[derive(Clone, Debug)]
struct ComputationContext<'a> {
    computer_config: &'a ComputerConfig,
    intermediate_representation: &'a IntermediateRepresentation,
    constants: rpds::VectorSync<Option<IntermediateValue>>,
    functions_results_cache: &'a Arc<RwLock<HashMap<u128, IntermediateValue>>>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct ComputerConfig {
    pub user_functions_caching: bool,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct Computer {
    pub config: ComputerConfig,
}

impl Computer {
    pub fn compute(
        &self,
        intermediate_representation: &IntermediateRepresentation,
    ) -> Result<Option<Value>> {
        let computation_context = ComputationContext {
            computer_config: &self.config,
            intermediate_representation,
            constants: rpds::VectorSync::from_iter(std::iter::repeat_n(
                None,
                intermediate_representation.unique_constants_names_count,
            )),
            functions_results_cache: &Arc::new(RwLock::new(HashMap::default())),
        };
        computation_context.unroll_intermediate_value(
            computation_context.compute_node(&intermediate_representation.root)?,
        )
    }
}

impl<'a> ComputationContext<'a> {
    fn compute_lazy_value(&self, lazy_value: &LazyValue) -> Result<IntermediateValue> {
        let lazy_value_computation_context = ComputationContext {
            computer_config: self.computer_config,
            intermediate_representation: self.intermediate_representation,
            constants: lazy_value.constants.clone(),
            functions_results_cache: self.functions_results_cache,
        };
        lazy_value_computation_context.compute_node(&lazy_value.node)
    }

    fn compute_next(
        &self,
        sequence: &Sequence,
        lockable_internals_write_guard: &mut parking_lot::RwLockWriteGuard<
            SequenceLockableInternals,
        >,
    ) -> Result<()> {
        let mut current_next_lazy_value =
            std::mem::take(&mut lockable_internals_write_guard.next_lazy_value).unwrap();
        let next = self.compute_lazy_value(&current_next_lazy_value)?;
        current_next_lazy_value.constants[sequence
            .intermediate_representation_content
            .current_constant_name_clustered_index] = Some(next.clone());
        lockable_internals_write_guard.next_lazy_value = Some(current_next_lazy_value);
        lockable_internals_write_guard
            .already_computed_values
            .push_back_mut(next.clone());
        Ok(())
    }

    fn get_from_sequence(&self, sequence: &Sequence, index: usize) -> Result<IntermediateValue> {
        let lockable_internals_read_guard = sequence.lockable_internals.upgradable_read();
        if let Some(result) = lockable_internals_read_guard
            .already_computed_values
            .inner
            .get(index)
        {
            Ok(result.clone())
        } else {
            let mut lockable_internals_write_guard =
                parking_lot::RwLockUpgradableReadGuard::upgrade(lockable_internals_read_guard);
            while lockable_internals_write_guard.already_computed_values.len() <= index {
                self.compute_next(sequence, &mut lockable_internals_write_guard)?;
            }
            Ok(lockable_internals_write_guard
                .already_computed_values
                .inner
                .last()
                .unwrap()
                .clone())
        }
    }

    fn get_range_from_sequence(
        &self,
        sequence: &Sequence,
        from: usize,
        to: usize,
    ) -> Result<List<IntermediateValue>> {
        let lockable_internals_read_guard = sequence.lockable_internals.upgradable_read();
        if lockable_internals_read_guard.already_computed_values.len() > to {
            Ok(List {
                inner: lockable_internals_read_guard
                    .already_computed_values
                    .inner
                    .iter()
                    .skip(from)
                    .take(to - from)
                    .collect(),
            })
        } else {
            let mut lockable_internals_write_guard =
                parking_lot::RwLockUpgradableReadGuard::upgrade(lockable_internals_read_guard);
            while lockable_internals_write_guard.already_computed_values.len() <= to {
                self.compute_next(sequence, &mut lockable_internals_write_guard)?;
            }
            Ok(List {
                inner: lockable_internals_write_guard
                    .already_computed_values
                    .inner
                    .iter()
                    .skip(from)
                    .take(to - from)
                    .collect(),
            })
        }
    }

    fn get_from_intermediate_value(
        &self,
        intermediate_value: &IntermediateValue,
        index: usize,
    ) -> Result<Option<IntermediateValue>> {
        match intermediate_value {
            IntermediateValue::Tuple(list) => Ok(list.inner.get(index).cloned()),
            IntermediateValue::Sequence(sequence) => {
                Ok(Some(self.get_from_sequence(sequence, index)?))
            }
            IntermediateValue::Value(Some(Value::Tuple(list))) => {
                Ok(list.inner.get(index).cloned().map(IntermediateValue::Value))
            }
            unexpected_value => Err(anyhow!(
                "expected tuple or sequence, found {:#?}",
                unexpected_value
            )),
        }
    }

    fn get_range_from_intermediate_value(
        &self,
        intermediate_value: &IntermediateValue,
        from: usize,
        to: usize,
    ) -> Result<List<IntermediateValue>> {
        match intermediate_value {
            IntermediateValue::Tuple(list) => Ok(List {
                inner: list.inner.iter().skip(from).take(to - from).collect(),
            }),
            IntermediateValue::Sequence(sequence) => {
                Ok(self.get_range_from_sequence(sequence, from, to)?)
            }
            IntermediateValue::Value(Some(Value::Tuple(list))) => Ok(List {
                inner: list
                    .inner
                    .iter()
                    .skip(from)
                    .take(to - from)
                    .cloned()
                    .map(IntermediateValue::Value)
                    .collect(),
            }),
            unexpected_value => Err(anyhow!(
                "expected tuple or sequence, found {:#?}",
                unexpected_value
            )),
        }
    }

    fn unroll_intermediate_value(
        &self,
        intermediate_value: IntermediateValue,
    ) -> Result<Option<Value>> {
        match intermediate_value {
            IntermediateValue::Value(result) => Ok(result),
            IntermediateValue::Tuple(intermediate_values_list) => {
                let mut result = List::default();
                for intermediate_value in intermediate_values_list.inner.into_iter() {
                    result.push_back_mut(self.unroll_intermediate_value(intermediate_value)?);
                }
                Ok(Some(Value::Tuple(result)))
            }
            IntermediateValue::Object(object) => {
                let mut result = Map::default();
                for (key, intermediate_value) in object.inner.into_iter() {
                    result.inner.insert_mut(
                        key.clone(),
                        self.unroll_intermediate_value(intermediate_value.clone())?,
                    );
                }
                Ok(Some(Value::Object(result)))
            }
            IntermediateValue::Sequence(_) => Err(anyhow!(
                "expected value, tuple or object, found unlimited sequence"
            )),
        }
    }

    fn compute_nodes<N>(
        &self,
        nodes_and_computation_contexts_iterator: N,
        nodes_count: usize,
    ) -> Result<Vec<IntermediateValue>>
    where
        N: Iterator<Item = (&'a Node, Cow<'a, Self>)>,
    {
        let mut result = vec![IntermediateValue::Value(None); nodes_count];
        let complex_elements = nodes_and_computation_contexts_iterator
            .enumerate()
            .filter(
                |(element_index, (node, computation_context))| match &node.content {
                    Content::Value(value) => {
                        result[*element_index] = IntermediateValue::Value(unsafe {
                            std::mem::transmute::<
                                Option<intermediate_representation::Value>,
                                Option<Value>,
                            >(value.clone())
                        });
                        false
                    }
                    Content::Constant(constant_name_clustered_index) => {
                        result[*element_index] = computation_context.constants
                            [*constant_name_clustered_index]
                            .clone()
                            .unwrap();
                        false
                    }
                    _ => true,
                },
            )
            .collect::<Vec<_>>();
        Ok(match complex_elements.len() {
            0 => result,
            1 => {
                let (element_index, (node, computation_context)) =
                    complex_elements.into_iter().next().unwrap();
                result[element_index] = computation_context.compute_node(node)?;
                result
            }
            2.. => {
                let result_mutex = Mutex::new(result);
                complex_elements
                    .into_par_iter()
                    .try_for_each(|(element_index, (node, computation_context))| {
                        computation_context.compute_node(node).map(|result| {
                            result_mutex.lock()[element_index] = result;
                        })
                    })
                    .map(|_| result_mutex.into_inner())?
            }
        })
    }

    fn compute_node(&self, node: &Node) -> Result<IntermediateValue> {
        match &node.content {
            Content::Tuple(tuple) => Ok(IntermediateValue::Tuple(List {
                inner: im_lists::list::SharedList::from_iter(self.compute_nodes(
                    tuple.iter().map(|node| (node, Cow::Borrowed(self))),
                    tuple.len(),
                )?),
            })),
            Content::Scope { constants, compute } => {
                let mut result_computation_context = self.clone();
                for (constant_name_clustered_index, computed_constant) in constants
                    .iter()
                    .map(|constant_definition| constant_definition.name_clustered_index)
                    .zip(self.compute_nodes(
                        constants.iter().map(|constant_definition| {
                            (&constant_definition.node, Cow::Borrowed(self))
                        }),
                        constants.len(),
                    )?)
                {
                    result_computation_context.constants[constant_name_clustered_index] =
                        Some(computed_constant);
                }
                result_computation_context.compute_node(compute)
            }
            Content::Constant(constant_name_clustered_index) => Ok(self.constants
                [*constant_name_clustered_index]
                .clone()
                .unwrap()),
            Content::EmbeddedFunctionCall {
                path,
                embedded_function,
            } => match &**embedded_function {
                EmbeddedFunction::Sum(argument) => {
                    Ok(IntermediateValue::Value(Some(Value::Number(
                        self.unroll_intermediate_value(self.compute_node(argument)?)?
                            .unwrap()
                            .as_tuple()
                            .unwrap()
                            .inner
                            .iter()
                            .fold(Rational::ZERO, |accumulator, current| {
                                accumulator + current.as_ref().unwrap().as_number().unwrap()
                            }),
                    ))))
                }
                EmbeddedFunction::IsSorted(argument) => {
                    Ok(IntermediateValue::Value(Some(Value::Bool(
                        self.unroll_intermediate_value(self.compute_node(argument)?)?
                            .unwrap()
                            .as_tuple()
                            .unwrap()
                            .inner
                            .iter()
                            .is_sorted(),
                    ))))
                }
                EmbeddedFunction::StandardInput => {
                    let mut result = String::new();
                    std::io::stdin()
                        .read_to_string(&mut result)
                        .with_context(|| {
                            format!("can not compute embedded function at path {:#?}", path)
                        })?;
                    Ok(IntermediateValue::Value(Some(Value::String(
                        ropey::Rope::from(result),
                    ))))
                }
                EmbeddedFunction::ParseYaml(argument) => Ok(IntermediateValue::Value(Some(
                    serde_saphyr::from_str::<Value>(
                        &self
                            .unroll_intermediate_value(self.compute_node(argument)?)?
                            .unwrap()
                            .as_string()
                            .unwrap()
                            .to_string(),
                    )
                    .with_context(|| {
                        format!("can not compute embedded function at path {:#?}", path)
                    })?,
                ))),
                EmbeddedFunction::KeyValuePairs(argument) => {
                    Ok(IntermediateValue::Value(Some(Value::Tuple(List {
                        inner: im_lists::list::SharedList::from_iter(
                            self.unroll_intermediate_value(self.compute_node(argument)?)?
                                .unwrap()
                                .as_object()
                                .unwrap()
                                .inner
                                .iter()
                                .map(|(key, value)| {
                                    Some(Value::Tuple(List {
                                        inner: im_lists::list::SharedList::from_iter([
                                            Some(Value::String(ropey::Rope::from_str(key))),
                                            value.clone(),
                                        ]),
                                    }))
                                }),
                        ),
                    }))))
                }
                EmbeddedFunction::Flatten(argument) => {
                    Ok(IntermediateValue::Value(Some(Value::Tuple({
                        List {
                            inner: im_lists::list::SharedList::from_iter(
                                self.unroll_intermediate_value(self.compute_node(argument)?)?
                                    .unwrap()
                                    .as_tuple()
                                    .unwrap()
                                    .inner
                                    .iter()
                                    .cloned()
                                    .flat_map(|list| {
                                        list.unwrap().as_tuple_mut().unwrap().inner.to_owned()
                                    }),
                            ),
                        }
                    }))))
                }
            },
            Content::UserFunctionCall { arguments, body } => {
                let mut result_computation_context = self.clone();
                for (constant_name_clustered_index, computed_constant) in arguments
                    .iter()
                    .map(|constant_definition| constant_definition.name_clustered_index)
                    .zip(self.compute_nodes(
                        arguments.iter().map(|constant_definition| {
                            (&constant_definition.node, Cow::Borrowed(self))
                        }),
                        arguments.len(),
                    )?)
                {
                    result_computation_context.constants[constant_name_clustered_index] =
                        Some(computed_constant);
                }
                let user_function = &self.intermediate_representation.user_functions[*body];
                if self.computer_config.user_functions_caching && user_function.is_pure {
                    let function_call_identifier = {
                        let mut hasher = gxhash::GxHasher::default();
                        for constant_name_clustered_index in
                            &user_function.external_constants_name_clustered_indices
                        {
                            let constant_value = &result_computation_context.constants
                                [*constant_name_clustered_index];
                            constant_value.hash(&mut hasher);
                        }
                        body.hash(&mut hasher);
                        hasher.finish_u128()
                    };
                    let functions_results_cache_read_guard = self.functions_results_cache.read();
                    if let Some(cached_function_result) =
                        functions_results_cache_read_guard.get(&function_call_identifier)
                    {
                        Ok(cached_function_result.clone())
                    } else {
                        drop(functions_results_cache_read_guard);
                        let result =
                            result_computation_context.compute_node(&user_function.node)?;
                        self.functions_results_cache
                            .write()
                            .insert(function_call_identifier, result.clone());
                        Ok(result)
                    }
                } else {
                    result_computation_context.compute_node(&user_function.node)
                }
            }
            Content::FromAt {
                from,
                value_path_segments,
            } => {
                let mut result = self.compute_node(from)?;
                for path_segment in value_path_segments {
                    match path_segment {
                        ValuePathSegment::ArrayIndex(array_index) => {
                            result = std::mem::take(
                                &mut self
                                    .get_from_intermediate_value(&result, *array_index)?
                                    .with_context(|| {
                                        format!(
                                            "expected array with element at index {array_index}, \
                                             found array {result:#?}"
                                        )
                                    })?,
                            )
                        }
                        ValuePathSegment::ObjectKey(object_key) => {
                            result = IntermediateValue::Value(std::mem::take(
                                self.unroll_intermediate_value(result)?
                                    .unwrap()
                                    .as_object_mut()
                                    .unwrap()
                                    .inner
                                    .get_mut(object_key)
                                    .unwrap(),
                            ))
                        }
                        ValuePathSegment::ArrayRange((from, to)) => {
                            let from_number = match from {
                                RangeBound::Static(Some(from)) => *from,
                                RangeBound::Static(None) => 0,
                                RangeBound::Dynamic(from_node) => {
                                    self.unroll_intermediate_value(self.compute_node(from_node)?)?
                                        .unwrap()
                                        .as_number()
                                        .unwrap()
                                        .to_f64()
                                        .value()
                                        .max(0f64) as usize
                                }
                            };
                            let to_number = match to {
                                RangeBound::Static(Some(to)) => *to,
                                RangeBound::Static(None) => 0,
                                RangeBound::Dynamic(to_node) => {
                                    self.unroll_intermediate_value(self.compute_node(to_node)?)?
                                        .unwrap()
                                        .as_number()
                                        .unwrap()
                                        .to_f64()
                                        .value()
                                        .max(0f64) as usize
                                }
                            };
                            result =
                                IntermediateValue::Tuple(self.get_range_from_intermediate_value(
                                    &std::mem::take(&mut result),
                                    from_number,
                                    to_number,
                                )?)
                        }
                    }
                }
                Ok(result)
            }
            Content::Match {
                r#match,
                cases,
                match_constant_name_clustered_index_option,
            } => {
                let computed_match = self.unroll_intermediate_value(self.compute_node(r#match)?)?;
                let match_type = Value::r#type(&computed_match);
                for case in cases {
                    match &case.condition {
                        Condition::Type(expected_type) => {
                            if expected_type.contains(&match_type) {
                                if let Some(match_constant_name_clustered_index) =
                                    match_constant_name_clustered_index_option
                                {
                                    let mut case_computation_context = self.clone();
                                    case_computation_context.constants
                                        [*match_constant_name_clustered_index] =
                                        Some(IntermediateValue::Value(computed_match));
                                    return case_computation_context.compute_node(&case.node);
                                } else {
                                    return self.compute_node(&case.node);
                                }
                            }
                        }
                        Condition::Value(expected_value_node) => {
                            let computed_expected_value = self.unroll_intermediate_value(
                                self.compute_node(expected_value_node)?,
                            )?;
                            if computed_expected_value == computed_match {
                                if let Some(match_constant_name_clustered_index) =
                                    match_constant_name_clustered_index_option
                                {
                                    let mut case_computation_context = self.clone();
                                    case_computation_context.constants
                                        [*match_constant_name_clustered_index] =
                                        Some(IntermediateValue::Value(computed_match));
                                    return case_computation_context.compute_node(&case.node);
                                } else {
                                    return self.compute_node(&case.node);
                                }
                            }
                        }
                    }
                }
                panic!("no case from {cases:#?} matches {computed_match:#?}")
            }
            Content::Map {
                map,
                throughs,
                map_constant_name_clustered_index,
            } => {
                let computed_map = self.unroll_intermediate_value(self.compute_node(map)?)?;
                let computed_map_array = computed_map.as_ref().unwrap().as_tuple().unwrap();
                match throughs {
                    Throughs::Array(node) => Ok(IntermediateValue::Tuple(List {
                        inner: im_lists::list::SharedList::from_iter(self.compute_nodes(
                            computed_map_array.inner.iter().map(|element_value| {
                                let mut through_computation_context = self.clone();
                                through_computation_context.constants
                                    [*map_constant_name_clustered_index] =
                                    Some(IntermediateValue::Value(element_value.clone()));
                                (&**node, Cow::Owned(through_computation_context))
                            }),
                            computed_map_array.inner.len(),
                        )?),
                    })),
                    Throughs::Tuple {
                        nodes_indexes,
                        nodes,
                    } => Ok(IntermediateValue::Tuple(List {
                        inner: im_lists::list::SharedList::from_iter(self.compute_nodes(
                            computed_map_array.inner.iter().enumerate().map(
                                |(element_index, element_value)| {
                                    let mut through_computation_context = self.clone();
                                    through_computation_context.constants
                                        [*map_constant_name_clustered_index] =
                                        Some(IntermediateValue::Value(element_value.clone()));
                                    (
                                        &nodes[nodes_indexes[element_index]],
                                        Cow::Owned(through_computation_context),
                                    )
                                },
                            ),
                            computed_map_array.inner.len(),
                        )?),
                    })),
                }
            }
            Content::Fold {
                fold,
                fold_constant_name_clustered_index,
                starting_with,
                accumulating_in_constant_name_clustered_index,
                throughs,
            } => {
                let computed_fold = self.unroll_intermediate_value(self.compute_node(fold)?)?;
                let computed_fold_array = computed_fold.as_ref().unwrap().as_tuple().unwrap();
                let mut result = self.compute_node(starting_with)?;
                match throughs {
                    Throughs::Array(through_node) => {
                        for element in computed_fold_array.inner.iter() {
                            let mut through_computation_context = self.clone();
                            through_computation_context.constants
                                [*fold_constant_name_clustered_index] =
                                Some(IntermediateValue::Value(element.clone()));
                            through_computation_context.constants
                                [*accumulating_in_constant_name_clustered_index] =
                                Some(result.clone());
                            result = through_computation_context.compute_node(through_node)?;
                        }
                    }
                    Throughs::Tuple {
                        nodes_indexes,
                        nodes,
                    } => {
                        for (element_index, element) in computed_fold_array.inner.iter().enumerate()
                        {
                            let mut through_computation_context = self.clone();
                            through_computation_context.constants
                                [*fold_constant_name_clustered_index] =
                                Some(IntermediateValue::Value(element.clone()));
                            through_computation_context.constants
                                [*accumulating_in_constant_name_clustered_index] =
                                Some(result.clone());
                            result = through_computation_context
                                .compute_node(&nodes[nodes_indexes[element_index]])?;
                        }
                    }
                }
                Ok(result)
            }
            Content::Sequence(intermediate_representation_content) => {
                let computed_starting_with =
                    self.compute_node(&intermediate_representation_content.starting_with)?;
                let mut next_constants = self.constants.clone();
                next_constants
                    [intermediate_representation_content.current_constant_name_clustered_index] =
                    Some(computed_starting_with.clone());
                Ok(IntermediateValue::Sequence(Sequence {
                    intermediate_representation_content: intermediate_representation_content
                        .clone(),
                    lockable_internals: Arc::new(RwLock::new(SequenceLockableInternals {
                        next_lazy_value: Some(LazyValue {
                            node: intermediate_representation_content.next.clone(),
                            constants: next_constants,
                        }),
                        already_computed_values: List {
                            inner: im_lists::list::SharedList::from_iter([computed_starting_with]),
                        },
                    })),
                }))
            }
            Content::Object(object) => Ok(IntermediateValue::Object(Map {
                inner: RedBlackTreeMapSync::from_iter(object.keys().cloned().zip(
                    self.compute_nodes(
                        object.values().map(|value| (value, Cow::Borrowed(self))),
                        object.len(),
                    )?,
                )),
            })),
            Content::Value(value) => Ok(unsafe {
                IntermediateValue::Value(std::mem::transmute::<
                    Option<intermediate_representation::Value>,
                    Option<Value>,
                >(value.clone()))
            }),
        }
    }
}
