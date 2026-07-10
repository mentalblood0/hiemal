use std::collections::BTreeMap;
use std::{borrow::Cow, hash::Hash, hash::Hasher, io::Read, sync::Arc};

use anyhow::{Context, Result, anyhow};
use dashu::Rational;
use gxhash::HashMap;
use parking_lot::{Mutex, RwLock};
use rpds::RedBlackTreeMapSync;
use serde::{Deserialize, Serialize};

use crate::intermediate_representation::{self, RangeBound, Throughs};
use crate::{
    containers::{self, List},
    intermediate_representation::{
        Condition, Content, EmbeddedFunction, IntermediateRepresentation, Node, ValuePathSegment,
    },
    value::Value,
};

type Constants = rpds::VectorSync<Option<IntermediateValue>>;

static THREADS_LEFT_TO_SPAWN: Mutex<u8> = Mutex::new(8u8);

fn compute_prepared_nodes<'a>(
    mut input: &[(usize, (&'a Node, Cow<'a, ComputationContext>))],
    output: &'a Mutex<Vec<IntermediateValue>>,
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
            let (element_index, (node, computation_context)) = &input[0];
            let node_result = computation_context.compute_node(node)?;
            output.lock()[*element_index] = node_result;
            return Ok(());
        } else {
            let left_half = input.split_off(..input.len().div_ceil(2)).unwrap();
            for (element_index, (node, computation_context)) in left_half {
                let node_result = computation_context.compute_node(node)?;
                output.lock()[*element_index] = node_result;
            }
        }
    }
    if !input.is_empty() {
        let left_half = input.split_off(..input.len().div_ceil(2)).unwrap();
        let (left_half_result, right_half_result) = std::thread::scope(|scope| {
            let left_half_join_handle = scope.spawn(|| compute_prepared_nodes(left_half, output));
            let right_half_result = compute_prepared_nodes(input, output);
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

#[derive(Clone, Debug)]
struct LazyValue {
    node: Arc<Node>,
    constants: Constants,
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

#[derive(Clone, Debug)]
struct MapLockableInternals {
    elements_taken_for_computation: BTreeMap<usize, Arc<RwLock<IntermediateValue>>>,
}

#[derive(Clone, Debug)]
struct Map {
    intermediate_representation_content: Arc<intermediate_representation::Map>,
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

#[repr(u8)]
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
enum IntermediateValue {
    Value(Option<Value>),
    Tuple(List<IntermediateValue>),
    Object(containers::Map<String, IntermediateValue>),
    Sequence(Sequence),
    Map(Map),
}

impl Default for IntermediateValue {
    fn default() -> Self {
        Self::Value(None)
    }
}

impl IntermediateValue {
    fn is_finite(&self) -> bool {
        match self {
            IntermediateValue::Value(_) => true,
            IntermediateValue::Tuple(tuple) => tuple.iter().all(|element| element.is_finite()),
            IntermediateValue::Object(object) => object.iter().all(|(_, value)| value.is_finite()),
            IntermediateValue::Sequence(_) => false,
            IntermediateValue::Map(Map {
                intermediate_representation_content,
                constants,
                lockable_internals: _,
            }) => constants[intermediate_representation_content.map_constant_name_clustered_index]
                .as_ref()
                .unwrap()
                .is_finite(),
        }
    }
}

#[derive(Clone, Debug)]
struct ComputationContext<'a> {
    computer_config: &'a ComputerConfig,
    intermediate_representation: &'a IntermediateRepresentation,
    constants: Constants,
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
            constants: Constants::from_iter(std::iter::repeat_n(
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

    fn get_from_intermediate_value(
        &self,
        intermediate_value: &IntermediateValue,
        index: usize,
    ) -> Result<Option<IntermediateValue>> {
        match intermediate_value {
            IntermediateValue::Tuple(list) => Ok(list.inner.get(index).cloned()),
            IntermediateValue::Sequence(sequence) => {
                let lockable_internals_read_guard = sequence.lockable_internals.upgradable_read();
                if let Some(result) = lockable_internals_read_guard
                    .already_computed_values
                    .inner
                    .get(index)
                {
                    Ok(Some(result.clone()))
                } else {
                    let mut lockable_internals_write_guard =
                        parking_lot::RwLockUpgradableReadGuard::upgrade(
                            lockable_internals_read_guard,
                        );
                    while lockable_internals_write_guard.already_computed_values.len() <= index {
                        self.compute_next(sequence, &mut lockable_internals_write_guard)?;
                    }
                    Ok(Some(
                        lockable_internals_write_guard
                            .already_computed_values
                            .inner
                            .last()
                            .unwrap()
                            .clone(),
                    ))
                }
            }
            IntermediateValue::Map(_) => Ok(self
                .get_range_from_intermediate_value(intermediate_value, index, index + 1)?
                .inner
                .into_iter()
                .next()),
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
                        parking_lot::RwLockUpgradableReadGuard::upgrade(
                            lockable_internals_read_guard,
                        );
                    while lockable_internals_write_guard.already_computed_values.len() < to {
                        self.compute_next(sequence, &mut lockable_internals_write_guard)?;
                    }
                    let result = Ok(List {
                        inner: lockable_internals_write_guard
                            .already_computed_values
                            .inner
                            .iter()
                            .skip(from)
                            .take(to - from)
                            .collect(),
                    });
                    drop(lockable_internals_write_guard);
                    result
                }
            }
            IntermediateValue::Map(map) => {
                let computed_map_range = self.get_range_from_intermediate_value(
                    map.constants[map
                        .intermediate_representation_content
                        .map_constant_name_clustered_index]
                        .as_ref()
                        .unwrap(),
                    from,
                    to,
                )?;
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
                        for element_index in from..to {
                            lockable_internals_write_guard
                                .elements_taken_for_computation
                                .entry(element_index)
                                .or_insert_with(|| {
                                    let element_value =
                                        Arc::new(RwLock::new(IntermediateValue::default()));
                                    to_compute.push((element_index, element_value.write_arc()));
                                    element_value
                                });
                        }
                    });
                    drop(lockable_internals_read_guard);
                    (already_taken, to_compute)
                };
                let mut already_taken_elements_iterator = already_taken_elements.iter();
                let mut already_taken_elements_iterator_current_option =
                    already_taken_elements_iterator.next();
                let mut result = self.compute_nodes(
                    computed_map_range.inner.iter().enumerate().map(
                        |(computed_map_element_index, computed_map_element)| {
                            if let Some((already_computed_through_index, _)) =
                                std::mem::take(&mut already_taken_elements_iterator_current_option)
                                && *already_computed_through_index
                                    == computed_map_element_index + from
                            {
                                already_taken_elements_iterator_current_option =
                                    already_taken_elements_iterator.next();
                                (None, Cow::Borrowed(self))
                            } else {
                                let mut through_computation_context = self.clone();
                                through_computation_context.constants[map
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
                                    Cow::Owned(through_computation_context),
                                )
                            }
                        },
                    ),
                    computed_map_range.inner.len(),
                )?;
                for (element_to_compute_index, mut element_to_compute_value) in
                    elements_to_compute.into_iter()
                {
                    *element_to_compute_value = result[element_to_compute_index - from].clone();
                    drop(element_to_compute_value);
                }
                let mut waiting_for_result_indexes = vec![false; result.len()];
                let mut results_to_wait = 0usize;
                for (already_taken_element_index, _) in already_taken_elements.iter() {
                    waiting_for_result_indexes[*already_taken_element_index - from] = true;
                    results_to_wait += 1;
                }
                for (already_computed_value_index, already_computed_value) in
                    already_taken_elements.iter().cycle()
                {
                    let result_index = already_computed_value_index - from;
                    if waiting_for_result_indexes[result_index]
                        && let Some(already_computed_value_read_guard) =
                            already_computed_value.try_read_recursive()
                    {
                        result[already_computed_value_index - from] =
                            already_computed_value_read_guard.clone();
                        drop(already_computed_value_read_guard);
                        waiting_for_result_indexes[result_index] = false;
                        results_to_wait -= 1;
                        if results_to_wait == 0 {
                            break;
                        }
                    }
                }
                Ok(List {
                    inner: result.into(),
                })
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
        if !intermediate_value.is_finite() {
            return Err(anyhow!(
                "expected finite intermediate value, got {intermediate_value:#?}"
            ));
        }
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
                let computed_map = self.get_range_from_intermediate_value(
                    map.constants[map
                        .intermediate_representation_content
                        .map_constant_name_clustered_index]
                        .as_ref()
                        .unwrap(),
                    0,
                    usize::MAX,
                )?;
                Ok(Some(Value::Tuple({
                    let mut result = List::default();
                    for element in self.compute_nodes(
                        computed_map.inner.iter().enumerate().map(
                            |(computed_map_element_index, computed_map_element)| {
                                let mut through_computation_context = self.clone();
                                through_computation_context.constants[map
                                    .intermediate_representation_content
                                    .map_constant_name_clustered_index] =
                                    Some(computed_map_element.clone());
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
                                    Cow::Owned(through_computation_context),
                                )
                            },
                        ),
                        computed_map.inner.len(),
                    )? {
                        result.push_back_mut(self.unroll_intermediate_value(element)?);
                    }
                    result
                })))
            }
            unexpected_variant => Err(anyhow!("unexpected enum variant {unexpected_variant:#?}")),
        }
    }

    fn compute_nodes<N>(
        &self,
        nodes_and_computation_contexts_iterator: N,
        nodes_count: usize,
    ) -> Result<Vec<IntermediateValue>>
    where
        N: Iterator<Item = (Option<&'a Node>, Cow<'a, Self>)>,
    {
        let mut result = vec![IntermediateValue::default(); nodes_count];
        let complex_elements = nodes_and_computation_contexts_iterator
            .enumerate()
            .filter_map(
                |(element_index, (node_or_intermediate_value, computation_context))| {
                    match node_or_intermediate_value {
                        Some(node) => match &node.content {
                            Content::Value(value) => {
                                result[element_index] = IntermediateValue::Value(unsafe {
                                    std::mem::transmute::<
                                        Option<intermediate_representation::Value>,
                                        Option<Value>,
                                    >(value.clone())
                                });
                                None
                            }
                            Content::Constant(constant_name_clustered_index) => {
                                result[element_index] = computation_context.constants
                                    [*constant_name_clustered_index]
                                    .clone()
                                    .unwrap();
                                None
                            }
                            _ => Some((element_index, (node, computation_context))),
                        },
                        None => None,
                    }
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
                compute_prepared_nodes(&complex_elements, &result_mutex)?;
                result_mutex.into_inner()
            }
        })
    }

    fn compute_node(&self, node: &Node) -> Result<IntermediateValue> {
        match &node.content {
            Content::Tuple(tuple) => Ok(IntermediateValue::Tuple(List {
                inner: im_lists::list::SharedList::from_iter(self.compute_nodes(
                    tuple.iter().map(|node| (Some(node), Cow::Borrowed(self))),
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
                            (Some(&constant_definition.node), Cow::Borrowed(self))
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
                            (Some(&constant_definition.node), Cow::Borrowed(self))
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
                    if let Some(cached_function_result) = self
                        .functions_results_cache
                        .read()
                        .get(&function_call_identifier)
                    {
                        Ok(cached_function_result.clone())
                    } else {
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
            Content::Map(intermediate_representation_content) => {
                let mut constants = self.constants.clone();
                constants[intermediate_representation_content.map_constant_name_clustered_index] =
                    Some(self.compute_node(&intermediate_representation_content.map)?);
                Ok(IntermediateValue::Map(Map {
                    intermediate_representation_content: intermediate_representation_content
                        .clone(),
                    constants,
                    lockable_internals: Arc::new(RwLock::new(MapLockableInternals {
                        elements_taken_for_computation: BTreeMap::new(),
                    })),
                }))
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
            Content::Object(object) => Ok(IntermediateValue::Object(containers::Map {
                inner: RedBlackTreeMapSync::from_iter(
                    object.keys().cloned().zip(
                        self.compute_nodes(
                            object
                                .values()
                                .map(|node| (Some(node), Cow::Borrowed(self))),
                            object.len(),
                        )?,
                    ),
                ),
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
