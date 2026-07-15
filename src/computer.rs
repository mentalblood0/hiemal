use std::cell::LazyCell;
use std::collections::BTreeMap;
use std::sync::LazyLock;
use std::{borrow::Cow, hash::Hash, hash::Hasher, io::Read, sync::Arc};

use anyhow::{Context, Result, anyhow};
use dashu::Rational;
use gxhash::HashMap;
use parking_lot::{Mutex, RwLock};
use rpds::RedBlackTreeMapSync;
use serde::{Deserialize, Serialize};

use crate::{
    containers::{self, List},
    intermediate_representation::{
        self, Condition, Content, EmbeddedFunction, IntermediateRepresentation, Node, RangeBound,
        Throughs, ValuePathSegment,
    },
    r#type::Type,
    value::Value,
};

type Constants = rpds::VectorSync<Option<IntermediateValueAndMetadata>>;

static THREADS_LEFT_TO_SPAWN: LazyLock<Mutex<u8>> = LazyLock::new(|| {
    Mutex::new(
        (std::thread::available_parallelism()
            .unwrap_or(std::num::NonZero::try_from(1usize).unwrap())
            .get()
            .div_ceil(2)
            - 1) as u8,
    )
});

#[derive(Clone, Debug)]
struct LazyValue {
    node: Arc<Node>,
    constants: Constants,
}

#[derive(Clone, Debug)]
struct SequenceLockableInternals {
    next_lazy_value: Option<LazyValue>,
    already_computed_values: Vec<IntermediateValueAndMetadata>,
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

#[derive(Clone, Debug)]
struct MapLockableInternals {
    elements_taken_for_computation: BTreeMap<usize, Arc<RwLock<IntermediateValueAndMetadata>>>,
}

#[derive(Clone, Debug)]
struct Map {
    intermediate_representation_content: Arc<intermediate_representation::Map>,
    computed_map: Box<IntermediateValueAndMetadata>,
    constants: Constants,
    lockable_internals: Arc<RwLock<MapLockableInternals>>,
}

impl PartialEq for Map {
    fn eq(&self, other: &Self) -> bool {
        self.intermediate_representation_content == other.intermediate_representation_content
    }
}

impl Eq for Map {}

impl PartialOrd for Map {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Map {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.intermediate_representation_content
            .cmp(&other.intermediate_representation_content)
    }
}

impl Hash for Map {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.intermediate_representation_content.hash(state);
    }
}

#[derive(Clone, Debug)]
struct FilterLockableInternals {
    already_processed_values_count: usize,
    already_computed_values: Vec<IntermediateValueAndMetadata>,
}

#[derive(Clone, Debug)]
struct Filter {
    intermediate_representation_content: Arc<intermediate_representation::Filter>,
    computed_filter: Box<IntermediateValueAndMetadata>,
    constants: Constants,
    lockable_internals: Arc<RwLock<FilterLockableInternals>>,
}

impl PartialEq for Filter {
    fn eq(&self, other: &Self) -> bool {
        self.intermediate_representation_content == other.intermediate_representation_content
    }
}

impl Eq for Filter {}

impl PartialOrd for Filter {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Filter {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.intermediate_representation_content
            .cmp(&other.intermediate_representation_content)
    }
}

impl Hash for Filter {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.intermediate_representation_content.hash(state);
    }
}

#[repr(u8)]
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
enum IntermediateValue {
    Value(Option<Value>),
    Tuple(List<IntermediateValue>),
    Object(containers::Map<String, IntermediateValue>),
    Sequence(Sequence),
    Map(Map),
    Filter(Filter),
}

impl Default for IntermediateValue {
    fn default() -> Self {
        Self::Value(None)
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug, Hash, Default)]
struct IntermediateValueAndMetadata {
    intermediate_value: IntermediateValue,
    r#type: Type,
}

#[derive(Clone, Debug)]
struct ComputationContext<'a> {
    computer_config: &'a ComputerConfig,
    intermediate_representation: &'a IntermediateRepresentation,
    functions_results_cache: &'a Arc<RwLock<HashMap<u128, IntermediateValueAndMetadata>>>,
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
            functions_results_cache: &Arc::new(RwLock::new(HashMap::default())),
        };
        let constants = Constants::from_iter(std::iter::repeat_n(
            None,
            intermediate_representation.unique_constants_names_count,
        ));
        computation_context.unroll_intermediate_value(
            computation_context
                .compute_node(&intermediate_representation.root, &constants)?
                .intermediate_value,
        )
    }
}

impl<'a> ComputationContext<'a> {
    fn compute_next_in_sequence(
        &self,
        sequence: &Sequence,
        lockable_internals_write_guard: &mut parking_lot::RwLockWriteGuard<
            SequenceLockableInternals,
        >,
    ) -> Result<()> {
        let mut current_next_lazy_value =
            std::mem::take(&mut lockable_internals_write_guard.next_lazy_value).unwrap();
        let next = self.compute_node(
            &current_next_lazy_value.node,
            &current_next_lazy_value.constants,
        )?;
        current_next_lazy_value.constants[sequence
            .intermediate_representation_content
            .current_constant_name_clustered_index] = Some(next.clone());
        lockable_internals_write_guard.next_lazy_value = Some(current_next_lazy_value);
        lockable_internals_write_guard
            .already_computed_values
            .push(next.clone());
        Ok(())
    }

    fn compute_next_in_filter(
        &self,
        filter: &Filter,
        lockable_internals_write_guard: &mut parking_lot::RwLockWriteGuard<FilterLockableInternals>,
    ) -> Result<bool> {
        let next_input_value_index = lockable_internals_write_guard.already_processed_values_count;
        if let Some(next_input_value) =
            self.get_from_intermediate_value(&filter.computed_filter, next_input_value_index)?
        {
            let mut next_constants = filter.constants.clone();
            next_constants[filter
                .intermediate_representation_content
                .filter_constant_name_clustered_index] = Some(next_input_value.clone());
            let next_through = match &filter.intermediate_representation_content.throughs {
                Throughs::Array(node) => &**node,
                Throughs::Tuple {
                    nodes_indexes,
                    nodes,
                } => &nodes[nodes_indexes[next_input_value_index]],
            };
            let computed_next_through = self.compute_node(next_through, &next_constants)?;
            if self
                .unroll_intermediate_value(computed_next_through.intermediate_value)?
                .unwrap()
                .as_bool()
                .unwrap()
            {
                lockable_internals_write_guard
                    .already_computed_values
                    .push(next_input_value.clone());
            }
            lockable_internals_write_guard.already_processed_values_count += 1;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn get_from_intermediate_value(
        &self,
        intermediate_value_and_metadata: &IntermediateValueAndMetadata,
        index: usize,
    ) -> Result<Option<IntermediateValueAndMetadata>> {
        let element_type = match &intermediate_value_and_metadata.r#type {
            Type::Array(element_type) => *element_type.clone(),
            Type::Tuple(elements_types) => elements_types[index].clone(),
            _ => panic!(),
        };
        match &intermediate_value_and_metadata.intermediate_value {
            IntermediateValue::Tuple(list) => {
                Ok(list
                    .inner
                    .get(index)
                    .cloned()
                    .map(|element| IntermediateValueAndMetadata {
                        intermediate_value: element,
                        r#type: element_type,
                    }))
            }
            IntermediateValue::Sequence(sequence) => {
                let lockable_internals_read_guard = sequence.lockable_internals.upgradable_read();
                if let Some(result) = lockable_internals_read_guard
                    .already_computed_values
                    .get(index)
                {
                    Ok(Some(result.clone()))
                } else {
                    let mut lockable_internals_write_guard =
                        parking_lot::RwLockUpgradableReadGuard::upgrade(
                            lockable_internals_read_guard,
                        );
                    while lockable_internals_write_guard.already_computed_values.len() <= index {
                        self.compute_next_in_sequence(
                            sequence,
                            &mut lockable_internals_write_guard,
                        )?;
                    }
                    Ok(Some(
                        lockable_internals_write_guard
                            .already_computed_values
                            .last()
                            .unwrap()
                            .clone(),
                    ))
                }
            }
            IntermediateValue::Filter(filter) => {
                let lockable_internals_read_guard = filter.lockable_internals.upgradable_read();
                if let Some(result) = lockable_internals_read_guard
                    .already_computed_values
                    .get(index)
                {
                    Ok(Some(result.clone()))
                } else {
                    let mut lockable_internals_write_guard =
                        parking_lot::RwLockUpgradableReadGuard::upgrade(
                            lockable_internals_read_guard,
                        );
                    while lockable_internals_write_guard.already_computed_values.len() <= index {
                        if !self
                            .compute_next_in_filter(filter, &mut lockable_internals_write_guard)?
                        {
                            return Ok(None);
                        }
                    }
                    Ok(Some(
                        lockable_internals_write_guard
                            .already_computed_values
                            .last()
                            .unwrap()
                            .clone(),
                    ))
                }
            }
            IntermediateValue::Map(_) => Ok(self
                .get_range_from_intermediate_value(
                    intermediate_value_and_metadata,
                    index,
                    index + 1,
                )?
                .into_iter()
                .next()),
            IntermediateValue::Value(Some(Value::Tuple(list))) => Ok(list
                .inner
                .get(index)
                .cloned()
                .map(IntermediateValue::Value)
                .map(|element| IntermediateValueAndMetadata {
                    intermediate_value: element,
                    r#type: element_type,
                })),
            unexpected_value => Err(anyhow!(
                "expected tuple, sequence, map or filter, found {:#?}",
                unexpected_value
            )),
        }
    }

    fn get_range_from_intermediate_value(
        &self,
        intermediate_value_and_metadata: &IntermediateValueAndMetadata,
        from: usize,
        to: usize,
    ) -> Result<Vec<IntermediateValueAndMetadata>> {
        let elements_types = match &intermediate_value_and_metadata.r#type {
            Type::Array(element_type) => [*element_type.clone()].to_vec(),
            Type::Tuple(elements_types) => elements_types
                .iter()
                .skip(from)
                .take(to - from)
                .cloned()
                .collect::<Vec<_>>(),
            _ => panic!(),
        };
        match &intermediate_value_and_metadata.intermediate_value {
            IntermediateValue::Tuple(list) => Ok(list
                .inner
                .iter()
                .skip(from)
                .take(to - from)
                .cloned()
                .zip(elements_types)
                .map(|(element, element_type)| IntermediateValueAndMetadata {
                    intermediate_value: element,
                    r#type: element_type,
                })
                .collect()),
            IntermediateValue::Sequence(sequence) => {
                let lockable_internals_read_guard = sequence.lockable_internals.upgradable_read();
                if lockable_internals_read_guard.already_computed_values.len() >= to {
                    Ok(lockable_internals_read_guard
                        .already_computed_values
                        .iter()
                        .skip(from)
                        .take(to - from)
                        .cloned()
                        .collect())
                } else {
                    let mut lockable_internals_write_guard =
                        parking_lot::RwLockUpgradableReadGuard::upgrade(
                            lockable_internals_read_guard,
                        );
                    while lockable_internals_write_guard.already_computed_values.len() < to {
                        self.compute_next_in_sequence(
                            sequence,
                            &mut lockable_internals_write_guard,
                        )?;
                    }
                    Ok(lockable_internals_write_guard
                        .already_computed_values
                        .iter()
                        .skip(from)
                        .take(to - from)
                        .cloned()
                        .collect())
                }
            }
            IntermediateValue::Filter(filter) => {
                let lockable_internals_read_guard = filter.lockable_internals.upgradable_read();
                if lockable_internals_read_guard.already_computed_values.len() >= to {
                    Ok(lockable_internals_read_guard
                        .already_computed_values
                        .iter()
                        .skip(from)
                        .take(to - from)
                        .cloned()
                        .collect())
                } else {
                    let mut lockable_internals_write_guard =
                        parking_lot::RwLockUpgradableReadGuard::upgrade(
                            lockable_internals_read_guard,
                        );
                    while lockable_internals_write_guard.already_computed_values.len() < to {
                        if !self
                            .compute_next_in_filter(filter, &mut lockable_internals_write_guard)?
                        {
                            break;
                        }
                    }
                    Ok(lockable_internals_write_guard
                        .already_computed_values
                        .iter()
                        .skip(from)
                        .take(to - from)
                        .cloned()
                        .collect())
                }
            }
            IntermediateValue::Map(map) => {
                let computed_map_range =
                    self.get_range_from_intermediate_value(&map.computed_map, from, to)?;
                let (already_taken_elements, elements_to_compute) = {
                    let mut lockable_internals_read_guard =
                        map.lockable_internals.upgradable_read();
                    let already_taken = lockable_internals_read_guard
                        .elements_taken_for_computation
                        .range(from..to)
                        .map(|(key, value)| (*key, value.clone()))
                        .collect::<Vec<_>>();
                    let mut to_compute = Vec::new();
                    lockable_internals_read_guard.with_upgraded(|lockable_internals_write_guard| {
                        for element_index in from..computed_map_range.len() {
                            lockable_internals_write_guard
                                .elements_taken_for_computation
                                .entry(element_index)
                                .or_insert_with(|| {
                                    let element_value = Arc::new(RwLock::new(
                                        IntermediateValueAndMetadata::default(),
                                    ));
                                    to_compute.push((element_index, element_value.write_arc()));
                                    element_value
                                });
                        }
                    });
                    (already_taken, to_compute)
                };
                let mut already_taken_elements_iterator = already_taken_elements.iter();
                let mut already_taken_elements_iterator_current_option =
                    already_taken_elements_iterator.next();
                let mut result = self.compute_nodes(
                    computed_map_range.iter().enumerate().map(
                        |(computed_map_element_index, computed_map_element)| {
                            if let Some((already_computed_through_index, _)) =
                                std::mem::take(&mut already_taken_elements_iterator_current_option)
                                && *already_computed_through_index
                                    == computed_map_element_index + from
                            {
                                already_taken_elements_iterator_current_option =
                                    already_taken_elements_iterator.next();
                                (None, Cow::Borrowed(&map.constants))
                            } else {
                                let mut through_constants = map.constants.clone();
                                through_constants[map
                                    .intermediate_representation_content
                                    .map_constant_name_clustered_index] =
                                    Some(computed_map_element.clone());
                                (
                                    Some(match &map.intermediate_representation_content.throughs {
                                        Throughs::Array(node) => node,
                                        Throughs::Tuple {
                                            nodes_indexes,
                                            nodes,
                                        } => {
                                            &nodes[nodes_indexes[computed_map_element_index + from]]
                                        }
                                    }),
                                    Cow::Owned(through_constants),
                                )
                            }
                        },
                    ),
                    computed_map_range.len(),
                )?;
                for (element_to_compute_index, mut element_to_compute_value) in
                    elements_to_compute.into_iter()
                {
                    *element_to_compute_value = result[element_to_compute_index - from].clone();
                }
                for (already_computed_value_index, already_computed_value) in
                    already_taken_elements.into_iter()
                {
                    result[already_computed_value_index - from] =
                        already_computed_value.read().clone();
                }
                Ok(result)
            }
            IntermediateValue::Value(Some(Value::Tuple(list))) => Ok(list
                .inner
                .iter()
                .skip(from)
                .take(to - from)
                .cloned()
                .zip(elements_types)
                .map(|(value, r#type)| IntermediateValueAndMetadata {
                    intermediate_value: IntermediateValue::Value(value),
                    r#type: r#type.clone(),
                })
                .collect()),
            unexpected_value => Err(anyhow!(
                "expected tuple, sequence, map or filter, found {:#?}",
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
                    result
                        .push_back_mut(self.unroll_intermediate_value(intermediate_value.clone())?);
                }
                Ok(Some(Value::Tuple(result)))
            }
            IntermediateValue::Object(object) => {
                let mut result = containers::Map::default();
                for (key, intermediate_value) in object.inner.into_iter() {
                    result.inner.insert_mut(
                        key.clone(),
                        self.unroll_intermediate_value(intermediate_value.clone())?,
                    );
                }
                Ok(Some(Value::Object(result)))
            }
            IntermediateValue::Map(map) => {
                let computed_map_range =
                    self.get_range_from_intermediate_value(&map.computed_map, 0, usize::MAX)?;
                let computed_map_len = computed_map_range.len();
                Ok(Some(Value::Tuple({
                    let mut result = List::default();
                    for element in self.compute_nodes(
                        computed_map_range.into_iter().enumerate().map(
                            |(computed_map_element_index, computed_map_element)| {
                                let mut through_constants = map.constants.clone();
                                through_constants[map
                                    .intermediate_representation_content
                                    .map_constant_name_clustered_index] =
                                    Some(computed_map_element);
                                (
                                    match &map.intermediate_representation_content.throughs {
                                        Throughs::Array(node) => Some(&**node),
                                        Throughs::Tuple {
                                            nodes_indexes,
                                            nodes,
                                        } => {
                                            Some(&nodes[nodes_indexes[computed_map_element_index]])
                                        }
                                    },
                                    Cow::Owned(through_constants),
                                )
                            },
                        ),
                        computed_map_len,
                    )? {
                        result.push_back_mut(
                            self.unroll_intermediate_value(element.intermediate_value)?,
                        );
                    }
                    result
                })))
            }
            unexpected_variant => Err(anyhow!("unexpected enum variant {unexpected_variant:#?}")),
        }
    }

    fn compute_prepared_nodes<'b>(
        &self,
        mut input: &[(usize, (&'b Node, Cow<'b, Constants>))],
        output: &'b Mutex<Vec<IntermediateValueAndMetadata>>,
    ) -> Result<()> {
        while !input.is_empty()
            && (input.len() < 2 || {
                let mut threads_left_to_spawn_lock_guard = THREADS_LEFT_TO_SPAWN.lock();
                if *threads_left_to_spawn_lock_guard == 0u8 {
                    true
                } else {
                    *threads_left_to_spawn_lock_guard -= 1;
                    false
                }
            })
        {
            if input.len() == 1 {
                let (element_index, (node, constants)) = &input[0];
                let node_result = self.compute_node(node, constants)?;
                output.lock()[*element_index] = node_result;
                return Ok(());
            } else {
                let left_half = input.split_off(..input.len().div_ceil(2)).unwrap();
                for (element_index, (node, constants)) in left_half {
                    let node_result = self.compute_node(node, constants)?;
                    output.lock()[*element_index] = node_result;
                }
            }
        }
        if !input.is_empty() {
            let left_half = input.split_off(..input.len().div_ceil(2)).unwrap();
            let (left_half_result, right_half_result) = std::thread::scope(|scope| {
                let left_half_join_handle =
                    scope.spawn(|| self.compute_prepared_nodes(left_half, output));
                let right_half_result = self.compute_prepared_nodes(input, output);
                let left_half_result = left_half_join_handle
                    .join()
                    .map_err(|error| anyhow!("Thread panicked: {error:?}"));
                *THREADS_LEFT_TO_SPAWN.lock() += 1;
                (left_half_result, right_half_result)
            });
            left_half_result??;
            right_half_result?;
        }
        Ok(())
    }

    fn compute_nodes<N>(
        &self,
        nodes_and_computation_contexts_iterator: N,
        nodes_count: usize,
    ) -> Result<Vec<IntermediateValueAndMetadata>>
    where
        N: Iterator<Item = (Option<&'a Node>, Cow<'a, Constants>)>,
    {
        let mut result = vec![IntermediateValueAndMetadata::default(); nodes_count];
        let complex_elements = nodes_and_computation_contexts_iterator
            .enumerate()
            .filter_map(
                |(element_index, (node_option, constants))| match node_option {
                    Some(node) => match &node.content {
                        Content::Value(value) => {
                            result[element_index] = IntermediateValueAndMetadata {
                                intermediate_value: IntermediateValue::Value(unsafe {
                                    std::mem::transmute::<
                                        Option<intermediate_representation::Value>,
                                        Option<Value>,
                                    >(value.clone())
                                }),
                                r#type: node.r#type.clone(),
                            };
                            None
                        }
                        Content::Constant(constant_name_clustered_index) => {
                            result[element_index] =
                                constants[*constant_name_clustered_index].clone().unwrap();
                            None
                        }
                        _ => Some((element_index, (node, constants))),
                    },
                    None => None,
                },
            )
            .collect::<Vec<_>>();
        Ok(match complex_elements.len() {
            0 => result,
            1 => {
                let (element_index, (node, constants)) =
                    complex_elements.into_iter().next().unwrap();
                result[element_index] = self.compute_node(node, &constants)?;
                result
            }
            2.. => {
                let result_mutex = Mutex::new(result);
                self.compute_prepared_nodes(&complex_elements, &result_mutex)?;
                result_mutex.into_inner()
            }
        })
    }

    fn compute_node(
        &self,
        node: &Node,
        constants: &Constants,
    ) -> Result<IntermediateValueAndMetadata> {
        match &node.content {
            Content::Tuple(tuple) => {
                let computed_elements = self.compute_nodes(
                    tuple
                        .iter()
                        .map(|node| (Some(node), Cow::Borrowed(constants))),
                    tuple.len(),
                )?;
                let r#type = Type::Tuple(
                    computed_elements
                        .iter()
                        .map(|computed_element| &computed_element.r#type)
                        .cloned()
                        .collect(),
                );
                Ok(IntermediateValueAndMetadata {
                    intermediate_value: IntermediateValue::Tuple(List {
                        inner: rpds::VectorSync::from_iter(
                            computed_elements
                                .into_iter()
                                .map(|computed_node| computed_node.intermediate_value),
                        ),
                    }),
                    r#type,
                })
            }
            Content::Scope {
                constants: scope_constants,
                compute,
            } => {
                let mut result_constants = constants.clone();
                for (constant_name_clustered_index, computed_constant) in scope_constants
                    .iter()
                    .map(|constant_definition| constant_definition.name_clustered_index)
                    .zip(self.compute_nodes(
                        scope_constants.iter().map(|constant_definition| {
                            (Some(&constant_definition.node), Cow::Borrowed(constants))
                        }),
                        scope_constants.len(),
                    )?)
                {
                    result_constants[constant_name_clustered_index] = Some(computed_constant);
                }
                self.compute_node(compute, &result_constants)
            }
            Content::Constant(constant_name_clustered_index) => {
                Ok(constants[*constant_name_clustered_index].clone().unwrap())
            }
            Content::EmbeddedFunctionCall {
                path,
                embedded_function,
            } => match &**embedded_function {
                EmbeddedFunction::Sum(argument) => Ok(IntermediateValueAndMetadata {
                    intermediate_value: IntermediateValue::Value(Some(Value::Number(
                        self.get_range_from_intermediate_value(
                            &self.compute_node(argument, constants)?,
                            0,
                            usize::MAX,
                        )?
                        .into_iter()
                        .fold(Rational::ZERO, |accumulator, current| {
                            accumulator
                                + self
                                    .unroll_intermediate_value(current.intermediate_value.clone())
                                    .unwrap()
                                    .as_ref()
                                    .unwrap()
                                    .as_number()
                                    .unwrap()
                        }),
                    ))),
                    r#type: node.r#type.clone(),
                }),
                EmbeddedFunction::IsSorted(argument) => Ok(IntermediateValueAndMetadata {
                    intermediate_value: IntermediateValue::Value(Some(Value::Bool(
                        self.get_range_from_intermediate_value(
                            &self.compute_node(argument, constants)?,
                            0,
                            usize::MAX,
                        )?
                        .is_sorted(),
                    ))),
                    r#type: node.r#type.clone(),
                }),
                EmbeddedFunction::StandardInput => {
                    let mut result = String::new();
                    std::io::stdin()
                        .read_to_string(&mut result)
                        .with_context(|| {
                            format!("can not compute embedded function at path {:#?}", path)
                        })?;
                    Ok(IntermediateValueAndMetadata {
                        intermediate_value: IntermediateValue::Value(Some(Value::String(
                            ropey::Rope::from(result),
                        ))),
                        r#type: node.r#type.clone(),
                    })
                }
                EmbeddedFunction::ParseYaml(argument) => {
                    let result_value = serde_saphyr::from_str::<Option<Value>>(
                        &self
                            .unroll_intermediate_value(
                                self.compute_node(argument, constants)?.intermediate_value,
                            )?
                            .unwrap()
                            .as_string()
                            .unwrap()
                            .to_string(),
                    )
                    .with_context(|| {
                        format!("can not compute embedded function at path {:#?}", path)
                    })?;
                    let r#type = Value::r#type(&result_value);
                    Ok(IntermediateValueAndMetadata {
                        intermediate_value: IntermediateValue::Value(result_value),
                        r#type,
                    })
                }
                EmbeddedFunction::KeyValuePairs(argument) => Ok(IntermediateValueAndMetadata {
                    intermediate_value: IntermediateValue::Value(Some(Value::Tuple({
                        let mut result = List::default();
                        for (key, value) in self
                            .unroll_intermediate_value(
                                self.compute_node(argument, constants)?.intermediate_value,
                            )?
                            .unwrap()
                            .as_object()
                            .unwrap()
                            .inner
                            .iter()
                        {
                            result.push_back_mut(Some(Value::Tuple(List {
                                inner: rpds::VectorSync::from_iter([
                                    Some(Value::String(ropey::Rope::from_str(key))),
                                    value.clone(),
                                ]),
                            })));
                        }
                        result
                    }))),
                    r#type: node.r#type.clone(),
                }),
                EmbeddedFunction::Flatten(argument) => Ok(IntermediateValueAndMetadata {
                    intermediate_value: IntermediateValue::Value(Some(Value::Tuple({
                        let mut result = List::default();
                        for list in self
                            .unroll_intermediate_value(
                                self.compute_node(argument, constants)?.intermediate_value,
                            )?
                            .unwrap()
                            .as_tuple()
                            .unwrap()
                            .inner
                            .iter()
                        {
                            result.append_mut(list.as_ref().unwrap().as_tuple().unwrap().clone());
                        }
                        result
                    }))),
                    r#type: node.r#type.clone(),
                }),
            },
            Content::UserFunctionCall { arguments, body } => {
                let mut result_constants = constants.clone();
                for (constant_name_clustered_index, computed_constant) in arguments
                    .iter()
                    .map(|constant_definition| constant_definition.name_clustered_index)
                    .zip(self.compute_nodes(
                        arguments.iter().map(|constant_definition| {
                            (Some(&constant_definition.node), Cow::Borrowed(constants))
                        }),
                        arguments.len(),
                    )?)
                {
                    result_constants[constant_name_clustered_index] = Some(computed_constant);
                }
                let user_function = &self.intermediate_representation.user_functions[*body];
                if self.computer_config.user_functions_caching && user_function.is_pure {
                    let function_call_identifier = {
                        let mut hasher = gxhash::GxHasher::default();
                        for constant_name_clustered_index in
                            &user_function.external_constants_name_clustered_indices
                        {
                            let constant_value = &result_constants[*constant_name_clustered_index];
                            constant_value.hash(&mut hasher);
                        }
                        body.hash(&mut hasher);
                        hasher.finish_u128()
                    };
                    if let Some(cached_function_result) = self
                        .functions_results_cache
                        .read()
                        .get(&function_call_identifier)
                    {
                        Ok(cached_function_result.clone())
                    } else {
                        let result = self.compute_node(&user_function.node, &result_constants)?;
                        self.functions_results_cache
                            .write()
                            .insert(function_call_identifier, result.clone());
                        Ok(result)
                    }
                } else {
                    self.compute_node(&user_function.node, &result_constants)
                }
            }
            Content::FromAt {
                from,
                value_path_segments,
            } => {
                let mut result = self.compute_node(from, constants)?;
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
                            result = IntermediateValueAndMetadata {
                                intermediate_value: IntermediateValue::Value(std::mem::take(
                                    self.unroll_intermediate_value(result.intermediate_value)?
                                        .unwrap()
                                        .as_object_mut()
                                        .unwrap()
                                        .inner
                                        .get_mut(object_key)
                                        .unwrap(),
                                )),
                                r#type: result
                                    .r#type
                                    .as_object()
                                    .unwrap()
                                    .get(object_key)
                                    .unwrap()
                                    .clone(),
                            }
                        }
                        ValuePathSegment::ArrayRange((from, to)) => {
                            let from_number = match from {
                                RangeBound::Static(Some(from)) => *from,
                                RangeBound::Static(None) => 0,
                                RangeBound::Dynamic(from_node) => {
                                    self.unroll_intermediate_value(
                                        self.compute_node(from_node, constants)?.intermediate_value,
                                    )?
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
                                    self.unroll_intermediate_value(
                                        self.compute_node(to_node, constants)?.intermediate_value,
                                    )?
                                    .unwrap()
                                    .as_number()
                                    .unwrap()
                                    .to_f64()
                                    .value()
                                    .max(0f64) as usize
                                }
                            };
                            let result_elements = self
                                .get_range_from_intermediate_value(
                                    &std::mem::take(&mut result),
                                    from_number,
                                    to_number,
                                )?
                                .into_iter()
                                .collect::<Vec<_>>();
                            let r#type = Type::Tuple(
                                result_elements
                                    .iter()
                                    .map(|element| &element.r#type)
                                    .cloned()
                                    .collect(),
                            );
                            result = IntermediateValueAndMetadata {
                                intermediate_value: IntermediateValue::Tuple(List {
                                    inner: result_elements
                                        .into_iter()
                                        .map(|element| element.intermediate_value)
                                        .collect(),
                                }),
                                r#type,
                            }
                        }
                    }
                }
                Ok(IntermediateValueAndMetadata {
                    intermediate_value: result.intermediate_value,
                    r#type: result.r#type,
                })
            }
            Content::Match {
                r#match,
                cases,
                match_constant_name_clustered_index_option,
            } => {
                let computed_match = self.compute_node(r#match, constants)?;
                let case_constants = if let Some(match_constant_name_clustered_index) =
                    match_constant_name_clustered_index_option
                {
                    let mut result = constants.clone();
                    result[*match_constant_name_clustered_index] = Some(computed_match.clone());
                    Cow::Owned(result)
                } else {
                    Cow::Borrowed(constants)
                };
                if cases.len() == 1 {
                    self.compute_node(&cases.first().unwrap().node, &case_constants)
                } else {
                    let computed_match_unrolled = LazyCell::new(|| {
                        self.unroll_intermediate_value(computed_match.intermediate_value)
                    });
                    for case in cases {
                        match &case.condition {
                            Condition::Type(expected_type) => {
                                if expected_type.contains(&computed_match.r#type) {
                                    return self.compute_node(&case.node, &case_constants);
                                }
                            }
                            Condition::Value(expected_value_node) => {
                                let computed_expected_value = self.unroll_intermediate_value(
                                    self.compute_node(expected_value_node, constants)?
                                        .intermediate_value,
                                )?;
                                if &computed_expected_value == {
                                    match &*computed_match_unrolled {
                                        Ok(result) => result,
                                        Err(error) => return Err(anyhow!(error.to_string())),
                                    }
                                } {
                                    return self.compute_node(&case.node, &case_constants);
                                }
                            }
                        }
                    }
                    panic!("no case from {cases:#?} matches computed value");
                }
            }
            Content::Map(intermediate_representation_content) => Ok(IntermediateValueAndMetadata {
                intermediate_value: IntermediateValue::Map(Map {
                    intermediate_representation_content: intermediate_representation_content
                        .clone(),
                    computed_map: Box::new(
                        self.compute_node(&intermediate_representation_content.map, constants)?,
                    ),
                    constants: constants.clone(),
                    lockable_internals: Arc::new(RwLock::new(MapLockableInternals {
                        elements_taken_for_computation: BTreeMap::new(),
                    })),
                }),
                r#type: node.r#type.clone(),
            }),
            Content::Filter(intermediate_representation_content) => {
                Ok(IntermediateValueAndMetadata {
                    intermediate_value: IntermediateValue::Filter(Filter {
                        intermediate_representation_content: intermediate_representation_content
                            .clone(),
                        computed_filter: Box::new(self.compute_node(
                            &intermediate_representation_content.filter,
                            constants,
                        )?),
                        constants: constants.clone(),
                        lockable_internals: Arc::new(RwLock::new(FilterLockableInternals {
                            already_processed_values_count: 0,
                            already_computed_values: Vec::new(),
                        })),
                    }),
                    r#type: node.r#type.clone(),
                })
            }
            Content::Fold {
                fold,
                fold_constant_name_clustered_index,
                starting_with,
                accumulating_in_constant_name_clustered_index,
                throughs,
            } => {
                let computed_fold = self.unroll_intermediate_value(
                    self.compute_node(fold, constants)?.intermediate_value,
                )?;
                let computed_fold_array = computed_fold.as_ref().unwrap().as_tuple().unwrap();
                let mut result = self.compute_node(starting_with, constants)?;
                match throughs {
                    Throughs::Array(through_node) => {
                        for element in computed_fold_array.inner.iter() {
                            let mut through_constants = constants.clone();
                            through_constants[*fold_constant_name_clustered_index] =
                                Some(IntermediateValueAndMetadata {
                                    intermediate_value: IntermediateValue::Value(element.clone()),
                                    r#type: Value::r#type(element),
                                });
                            through_constants[*accumulating_in_constant_name_clustered_index] =
                                Some(result.clone());
                            result = self.compute_node(through_node, &through_constants)?;
                        }
                    }
                    Throughs::Tuple {
                        nodes_indexes,
                        nodes,
                    } => {
                        for (element_index, element) in computed_fold_array.inner.iter().enumerate()
                        {
                            let mut through_constants = constants.clone();
                            through_constants[*fold_constant_name_clustered_index] =
                                Some(IntermediateValueAndMetadata {
                                    intermediate_value: IntermediateValue::Value(element.clone()),
                                    r#type: node.r#type.clone(),
                                });
                            through_constants[*accumulating_in_constant_name_clustered_index] =
                                Some(result.clone());
                            result = self.compute_node(
                                &nodes[nodes_indexes[element_index]],
                                &through_constants,
                            )?;
                        }
                    }
                }
                Ok(result)
            }
            Content::Sequence(intermediate_representation_content) => {
                let computed_starting_with = self.compute_node(
                    &intermediate_representation_content.starting_with,
                    constants,
                )?;
                let mut next_constants = constants.clone();
                next_constants
                    [intermediate_representation_content.current_constant_name_clustered_index] =
                    Some(computed_starting_with.clone());
                Ok(IntermediateValueAndMetadata {
                    intermediate_value: IntermediateValue::Sequence(Sequence {
                        intermediate_representation_content: intermediate_representation_content
                            .clone(),
                        lockable_internals: Arc::new(RwLock::new(SequenceLockableInternals {
                            next_lazy_value: Some(LazyValue {
                                node: intermediate_representation_content.next.clone(),
                                constants: next_constants,
                            }),
                            already_computed_values: [computed_starting_with].into(),
                        })),
                    }),
                    r#type: node.r#type.clone(),
                })
            }
            Content::Object(object) => {
                let computed_values = self.compute_nodes(
                    object
                        .values()
                        .map(|node| (Some(node), Cow::Borrowed(constants))),
                    object.len(),
                )?;
                let r#type = Type::Object(BTreeMap::from_iter(
                    object.keys().cloned().zip(
                        computed_values
                            .iter()
                            .map(|computed_value| &computed_value.r#type)
                            .cloned(),
                    ),
                ));
                Ok(IntermediateValueAndMetadata {
                    intermediate_value: IntermediateValue::Object(containers::Map {
                        inner: RedBlackTreeMapSync::from_iter(
                            object.keys().cloned().zip(
                                computed_values
                                    .into_iter()
                                    .map(|computed_value| computed_value.intermediate_value),
                            ),
                        ),
                    }),
                    r#type,
                })
            }
            Content::Value(value) => Ok(IntermediateValueAndMetadata {
                intermediate_value: unsafe {
                    IntermediateValue::Value(std::mem::transmute::<
                        Option<intermediate_representation::Value>,
                        Option<Value>,
                    >(value.clone()))
                },
                r#type: node.r#type.clone(),
            }),
        }
    }
}
