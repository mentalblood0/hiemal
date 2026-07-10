use std::collections::BTreeMap;
use std::sync::LazyLock;
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

#[derive(Clone, Debug)]
struct ComputationContext<'a> {
    computer_config: &'a ComputerConfig,
    intermediate_representation: &'a IntermediateRepresentation,
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
            functions_results_cache: &Arc::new(RwLock::new(HashMap::default())),
        };
        let constants = Constants::from_iter(std::iter::repeat_n(
            None,
            intermediate_representation.unique_constants_names_count,
        ));
        computation_context.unroll_intermediate_value(
            computation_context.compute_node(&intermediate_representation.root, &constants)?,
            &constants,
        )
    }
}

impl<'a> ComputationContext<'a> {
    fn compute_next(
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
            .push_back_mut(next.clone());
        Ok(())
    }

    fn get_from_intermediate_value(
        &self,
        intermediate_value: &IntermediateValue,
        index: usize,
        constants: &Constants,
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
                .get_range_from_intermediate_value(intermediate_value, index, index + 1, constants)?
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
        constants: &Constants,
    ) -> Result<Vec<IntermediateValue>> {
        match intermediate_value {
            IntermediateValue::Tuple(list) => Ok(list
                .inner
                .iter()
                .skip(from)
                .take(to - from)
                .cloned()
                .collect()),
            IntermediateValue::Sequence(sequence) => {
                let lockable_internals_read_guard = sequence.lockable_internals.upgradable_read();
                if lockable_internals_read_guard.already_computed_values.len() > to {
                    Ok(lockable_internals_read_guard
                        .already_computed_values
                        .inner
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
                        self.compute_next(sequence, &mut lockable_internals_write_guard)?;
                    }
                    Ok(lockable_internals_write_guard
                        .already_computed_values
                        .inner
                        .iter()
                        .skip(from)
                        .take(to - from)
                        .cloned()
                        .collect())
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
                    constants,
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
                        for element_index in from..computed_map_range.len() {
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
                                (None, Cow::Borrowed(constants))
                            } else {
                                let mut through_constants = constants.clone();
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
                .map(IntermediateValue::Value)
                .collect()),
            unexpected_value => Err(anyhow!(
                "expected tuple or sequence, found {:#?}",
                unexpected_value
            )),
        }
    }

    fn unroll_intermediate_value(
        &self,
        intermediate_value: IntermediateValue,
        constants: &Constants,
    ) -> Result<Option<Value>> {
        match intermediate_value {
            IntermediateValue::Value(result) => Ok(result),
            IntermediateValue::Tuple(intermediate_values_list) => {
                let mut result = List::default();
                for intermediate_value in intermediate_values_list.inner.into_iter() {
                    result.push_back_mut(
                        self.unroll_intermediate_value(intermediate_value, constants)?,
                    );
                }
                Ok(Some(Value::Tuple(result)))
            }
            IntermediateValue::Object(object) => {
                let mut result = containers::Map::default();
                for (key, intermediate_value) in object.inner.into_iter() {
                    result.inner.insert_mut(
                        key.clone(),
                        self.unroll_intermediate_value(intermediate_value.clone(), constants)?,
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
                    constants,
                )?;
                let computed_map_len = computed_map.len();
                Ok(Some(Value::Tuple({
                    let mut result = List::default();
                    for element in self.compute_nodes(
                        computed_map.into_iter().enumerate().map(
                            |(computed_map_element_index, computed_map_element)| {
                                let mut through_constants = constants.clone();
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
                        result.push_back_mut(self.unroll_intermediate_value(element, constants)?);
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
        output: &'b Mutex<Vec<IntermediateValue>>,
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
    ) -> Result<Vec<IntermediateValue>>
    where
        N: Iterator<Item = (Option<&'a Node>, Cow<'a, Constants>)>,
    {
        let mut result = vec![IntermediateValue::default(); nodes_count];
        let complex_elements = nodes_and_computation_contexts_iterator
            .enumerate()
            .filter_map(|(element_index, (node_or_intermediate_value, constants))| {
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
                            result[element_index] =
                                constants[*constant_name_clustered_index].clone().unwrap();
                            None
                        }
                        _ => Some((element_index, (node, constants))),
                    },
                    None => None,
                }
            })
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

    fn compute_node(&self, node: &Node, constants: &Constants) -> Result<IntermediateValue> {
        match &node.content {
            Content::Tuple(tuple) => Ok(IntermediateValue::Tuple(List {
                inner: im_lists::list::SharedList::from_iter(
                    self.compute_nodes(
                        tuple
                            .iter()
                            .map(|node| (Some(node), Cow::Borrowed(constants))),
                        tuple.len(),
                    )?,
                ),
            })),
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
                EmbeddedFunction::Sum(argument) => {
                    Ok(IntermediateValue::Value(Some(Value::Number(
                        self.get_range_from_intermediate_value(
                            &self.compute_node(argument, constants)?,
                            0,
                            usize::MAX,
                            constants,
                        )?
                        .into_iter()
                        .fold(Rational::ZERO, |accumulator, current| {
                            accumulator
                                + self
                                    .unroll_intermediate_value(current.clone(), constants)
                                    .unwrap()
                                    .as_ref()
                                    .unwrap()
                                    .as_number()
                                    .unwrap()
                        }),
                    ))))
                }
                EmbeddedFunction::IsSorted(argument) => {
                    Ok(IntermediateValue::Value(Some(Value::Bool(
                        self.get_range_from_intermediate_value(
                            &self.compute_node(argument, constants)?,
                            0,
                            usize::MAX,
                            constants,
                        )?
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
                            .unroll_intermediate_value(
                                self.compute_node(argument, constants)?,
                                constants,
                            )?
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
                            self.unroll_intermediate_value(
                                self.compute_node(argument, constants)?,
                                constants,
                            )?
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
                                self.unroll_intermediate_value(
                                    self.compute_node(argument, constants)?,
                                    constants,
                                )?
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
                                    .get_from_intermediate_value(&result, *array_index, constants)?
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
                                self.unroll_intermediate_value(result, constants)?
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
                                    self.unroll_intermediate_value(
                                        self.compute_node(from_node, constants)?,
                                        constants,
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
                                        self.compute_node(to_node, constants)?,
                                        constants,
                                    )?
                                    .unwrap()
                                    .as_number()
                                    .unwrap()
                                    .to_f64()
                                    .value()
                                    .max(0f64) as usize
                                }
                            };
                            result = IntermediateValue::Tuple(List {
                                inner: self
                                    .get_range_from_intermediate_value(
                                        &std::mem::take(&mut result),
                                        from_number,
                                        to_number,
                                        constants,
                                    )?
                                    .into_iter()
                                    .collect(),
                            })
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
                let computed_match = self
                    .unroll_intermediate_value(self.compute_node(r#match, constants)?, constants)?;
                let match_type = Value::r#type(&computed_match);
                let case_constants = if let Some(match_constant_name_clustered_index) =
                    match_constant_name_clustered_index_option
                {
                    let mut result = constants.clone();
                    result[*match_constant_name_clustered_index] =
                        Some(IntermediateValue::Value(computed_match.clone()));
                    Cow::Owned(result)
                } else {
                    Cow::Borrowed(constants)
                };
                for case in cases {
                    match &case.condition {
                        Condition::Type(expected_type) => {
                            if expected_type.contains(&match_type) {
                                return self.compute_node(&case.node, &case_constants);
                            }
                        }
                        Condition::Value(expected_value_node) => {
                            let computed_expected_value = self.unroll_intermediate_value(
                                self.compute_node(expected_value_node, constants)?,
                                constants,
                            )?;
                            if computed_expected_value == computed_match {
                                return self.compute_node(&case.node, &case_constants);
                            }
                        }
                    }
                }
                panic!("no case from {cases:#?} matches computed value")
            }
            Content::Map(intermediate_representation_content) => {
                let mut map_constants = constants.clone();
                map_constants
                    [intermediate_representation_content.map_constant_name_clustered_index] =
                    Some(self.compute_node(&intermediate_representation_content.map, constants)?);
                Ok(IntermediateValue::Map(Map {
                    intermediate_representation_content: intermediate_representation_content
                        .clone(),
                    constants: map_constants,
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
                let computed_fold =
                    self.unroll_intermediate_value(self.compute_node(fold, constants)?, constants)?;
                let computed_fold_array = computed_fold.as_ref().unwrap().as_tuple().unwrap();
                let mut result = self.compute_node(starting_with, constants)?;
                match throughs {
                    Throughs::Array(through_node) => {
                        for element in computed_fold_array.inner.iter() {
                            let mut through_constants = constants.clone();
                            through_constants[*fold_constant_name_clustered_index] =
                                Some(IntermediateValue::Value(element.clone()));
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
                                Some(IntermediateValue::Value(element.clone()));
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
                                .map(|node| (Some(node), Cow::Borrowed(constants))),
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
