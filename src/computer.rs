use std::{
    borrow::Cow,
    cell::LazyCell,
    collections::BTreeMap,
    hash::{Hash, Hasher},
    io::{BufReader, BufWriter, Read, Write},
    ops::Range,
    str::FromStr,
    sync::{Arc, LazyLock},
};

use anyhow::{Context, Result, anyhow};
use bytes::Bytes;
use dashu::Rational;
use glob::glob;
use gxhash::HashMap;
use parking_lot::{Mutex, RwLock};
use regex::{Regex, escape};
use serde::{Deserialize, Serialize};
use sha2::Digest;
use ureq::{
    ProxyProtocol,
    config::{AutoHeaderValue, IpFamily, RedirectAuthHeaders},
};

use crate::{
    containers::{self, Object, Vector},
    embedded_functions::{EMBEDDED_FUNCTIONS, EmbeddedFunction},
    intermediate_representation::{
        self, Condition, Content, IntermediateRepresentation, Node, RangeBound, Throughs,
        ValuePathSegment,
    },
    program::Path,
    r#type::{Embedded, TypeKind},
    value::{Identified, Runtime, Value},
};

type Constants = rpds::VectorSync<Option<Arc<IntermediateValueAndMetadata>>>;

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
struct SequenceLockableInternals {
    current_concrete_type_and_next: Arc<Vec<(TypeKind, Arc<Node>)>>,
    next_constants: Constants,
    already_computed_values: Vec<Arc<IntermediateValueAndMetadata>>,
    already_fully_computed: bool,
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
    elements_taken_for_computation: BTreeMap<usize, Arc<RwLock<Arc<IntermediateValueAndMetadata>>>>,
}

#[derive(Clone, Debug)]
struct Map {
    intermediate_representation_content: Arc<intermediate_representation::Map>,
    computed_map: Arc<IntermediateValueAndMetadata>,
    throughs: Throughs,
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
    already_computed_values: Vec<Arc<IntermediateValueAndMetadata>>,
}

#[derive(Clone, Debug)]
struct Filter {
    intermediate_representation_content: Arc<intermediate_representation::Filter>,
    computed_filter: Arc<IntermediateValueAndMetadata>,
    throughs: Throughs,
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

#[derive(Clone, Debug, Default)]
struct FlattenLockableInternals {
    next_element_index: usize,
    next_element_element_index: usize,
    already_computed_values: Vec<Arc<IntermediateValueAndMetadata>>,
}

#[derive(Clone, Debug)]
struct Flatten {
    computed_flatten: Arc<IntermediateValueAndMetadata>,
    lockable_internals: Arc<RwLock<FlattenLockableInternals>>,
}

impl PartialEq for Flatten {
    fn eq(&self, other: &Self) -> bool {
        self.computed_flatten == other.computed_flatten
    }
}

impl Eq for Flatten {}

impl PartialOrd for Flatten {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Flatten {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.computed_flatten.cmp(&other.computed_flatten)
    }
}

impl Hash for Flatten {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.computed_flatten.hash(state);
    }
}

#[derive(Clone, Debug)]
struct LazyValue {
    node: Arc<Node>,
    constants: Constants,
    computed: Arc<Mutex<Option<Arc<IntermediateValueAndMetadata>>>>,
}

impl PartialEq for LazyValue {
    fn eq(&self, other: &Self) -> bool {
        (&self.node, &self.constants) == (&other.node, &other.constants)
    }
}

impl Eq for LazyValue {}

impl PartialOrd for LazyValue {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for LazyValue {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (&self.node, &self.constants).cmp(&(&other.node, &other.constants))
    }
}

impl Hash for LazyValue {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.node.hash(state);
        self.constants.hash(state);
    }
}

#[repr(u8)]
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
enum IntermediateValue {
    Value(Arc<Option<Value>>),
    Tuple(Vector<IntermediateValueAndMetadata>),
    Object(containers::Object<String, IntermediateValueAndMetadata>),
    Sequence(Sequence),
    Map(Map),
    Filter(Filter),
    Flatten(Flatten),
    LazyValue(LazyValue),
}

impl Default for IntermediateValue {
    fn default() -> Self {
        Self::Value(Arc::new(None))
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug, Hash, Default)]
struct IntermediateValueAndMetadata {
    intermediate_value: IntermediateValue,
    type_kind: TypeKind,
}

#[derive(Clone, Debug)]
struct ComputationContext<'a> {
    computer_config: &'a ComputerConfig,
    intermediate_representation: &'a IntermediateRepresentation,
    functions_results_cache: &'a Arc<RwLock<HashMap<u128, Arc<IntermediateValueAndMetadata>>>>,
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
    ) -> Result<Arc<Option<Value>>> {
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
            &computation_context.compute_node(&intermediate_representation.root, &constants)?,
        )
    }
}

fn concrete_or_else<F>(concrete: &TypeKind, or_else: F) -> TypeKind
where
    F: Fn() -> TypeKind,
{
    if concrete.is_concrete() {
        concrete.clone()
    } else {
        or_else()
    }
}

fn embedded_function_error_context(path_option: &Option<Path>) -> impl FnOnce() -> String {
    || {
        format!(
            "can not compute embedded function at path\n{}",
            serde_saphyr::to_string(path_option.as_ref().unwrap()).unwrap()
        )
    }
}

impl<'a> ComputationContext<'a> {
    fn build_regex(&self, source: &Vector<Option<Value>>) -> String {
        let mut result = String::new();
        for source_part in source.iter() {
            match source_part.as_ref().as_ref().unwrap() {
                Value::String(string) if string == "character" => {
                    result.push('.');
                }
                Value::String(string) if string == "whitespace character" => {
                    result.push_str("\\s");
                }
                Value::String(string) if string == "non-whitespace character" => {
                    result.push_str("\\S");
                }
                Value::String(string) if string == "digit" => {
                    result.push_str("\\d");
                }
                Value::String(string) if string == "non-digit" => {
                    result.push_str("\\D");
                }
                Value::String(string) if string == "word character" => {
                    result.push_str("\\w");
                }
                Value::String(string) if string == "non-word character" => {
                    result.push_str("\\W");
                }
                Value::String(string) if string == "start of string" => {
                    result.push('^');
                }
                Value::String(string) if string == "end of string" => {
                    result.push('$');
                }
                Value::String(string) if string == "word boundary" => {
                    result.push_str("\\b");
                }
                Value::String(string) if string == "non-word boundary" => {
                    result.push_str("\\B");
                }
                Value::Object(object) => {
                    if let Some(variants) = object.get(&"one of".to_string()) {
                        result.push_str("(?:");
                        result.push_str(
                            &variants
                                .as_ref()
                                .as_ref()
                                .unwrap()
                                .as_tuple()
                                .unwrap()
                                .iter()
                                .map(|variant| {
                                    self.build_regex(
                                        variant.as_ref().as_ref().unwrap().as_tuple().unwrap(),
                                    )
                                })
                                .collect::<Vec<_>>()
                                .join("|"),
                        );
                        result.push(')');
                    } else if let Some(string_of_characters_to_except) =
                        object.get(&"character except from string".to_string())
                    {
                        result.push_str("[^");
                        result.push_str(&escape(
                            &string_of_characters_to_except
                                .as_ref()
                                .as_ref()
                                .unwrap()
                                .as_string()
                                .unwrap()
                                .to_string(),
                        ));
                        result.push(']');
                    } else if let Some(raw_string) = object.get(&"raw string".to_string()) {
                        result.push_str(&escape(
                            &raw_string
                                .as_ref()
                                .as_ref()
                                .unwrap()
                                .as_string()
                                .unwrap()
                                .to_string(),
                        ));
                    } else if let (Some(group), Some(name)) = (
                        object.get(&"group".to_string()),
                        object.get(&"name".to_string()),
                    ) {
                        result.push_str("(?P<");
                        result.push_str(
                            &name
                                .as_ref()
                                .as_ref()
                                .unwrap()
                                .as_string()
                                .unwrap()
                                .to_string(),
                        );
                        result.push('>');
                        result.push_str(
                            &self.build_regex(group.as_ref().as_ref().unwrap().as_tuple().unwrap()),
                        );
                        result.push(')');
                    } else if let (Some(repeat), min, max, exactly) = (
                        object.get(&"repeat".to_string()),
                        object.get(&"min".to_string()),
                        object.get(&"max".to_string()),
                        object.get(&"exactly".to_string()),
                    ) {
                        result.push_str("(?:");
                        result
                            .push_str(&self.build_regex(
                                repeat.as_ref().as_ref().unwrap().as_tuple().unwrap(),
                            ));
                        result.push_str("){");
                        if let Some(exactly) = exactly {
                            result.push_str(
                                &(exactly
                                    .as_ref()
                                    .as_ref()
                                    .unwrap()
                                    .as_number()
                                    .unwrap()
                                    .trunc()
                                    .to_f64()
                                    .value() as i64)
                                    .max(0)
                                    .to_string(),
                            );
                        } else {
                            let min_number = if let Some(min) = min {
                                (min.as_ref()
                                    .as_ref()
                                    .unwrap()
                                    .as_number()
                                    .unwrap()
                                    .trunc()
                                    .to_f64()
                                    .value() as i64)
                                    .max(0)
                            } else {
                                0
                            };
                            result.push_str(&min_number.to_string());
                            result.push(',');
                            if let Some(max) = max {
                                result.push_str(
                                    &(max
                                        .as_ref()
                                        .as_ref()
                                        .unwrap()
                                        .as_number()
                                        .unwrap()
                                        .trunc()
                                        .to_f64()
                                        .value() as i64)
                                        .max(min_number)
                                        .to_string(),
                                );
                            }
                        }
                        result.push('}');
                    } else {
                        panic!()
                    }
                }
                _ => panic!(),
            }
        }
        result
    }

    fn compute_lazy_value(
        &self,
        lazy_value: &LazyValue,
    ) -> Result<Arc<IntermediateValueAndMetadata>> {
        let mut computed_lock = lazy_value.computed.lock();
        if computed_lock.is_none() {
            let result = self.compute_node(&lazy_value.node, &lazy_value.constants)?;
            *computed_lock = Some(result.clone());
            Ok(result)
        } else {
            Ok(computed_lock.clone().unwrap())
        }
    }

    fn with_computed_lazy_value<F, R>(&self, lazy_value: &LazyValue, function: F) -> Result<R>
    where
        F: Fn(&Arc<IntermediateValueAndMetadata>) -> Result<R>,
    {
        let mut computed_lock = lazy_value.computed.lock();
        if computed_lock.is_none() {
            let result = self.compute_node(&lazy_value.node, &lazy_value.constants)?;
            *computed_lock = Some(result);
        }
        function(computed_lock.as_ref().unwrap())
    }

    fn compute_next_in_sequence(
        &self,
        sequence: &Sequence,
        lockable_internals_write_guard: &mut parking_lot::RwLockWriteGuard<
            SequenceLockableInternals,
        >,
    ) -> Result<()> {
        if !lockable_internals_write_guard.already_fully_computed {
            let next_node = lockable_internals_write_guard
                .current_concrete_type_and_next
                .iter()
                .find_map(|(current_concrete_type_kind, possible_next_node)| {
                    if current_concrete_type_kind.contains(
                        &lockable_internals_write_guard
                            .already_computed_values
                            .last()
                            .unwrap()
                            .type_kind,
                    ) {
                        Some(possible_next_node)
                    } else {
                        None
                    }
                })
                .unwrap();
            let next =
                self.compute_node(next_node, &lockable_internals_write_guard.next_constants)?;
            lockable_internals_write_guard.next_constants[sequence
                .intermediate_representation_content
                .current_constant_name_clustered_index] = Some(next.clone());
            if let Some(ref r#while) = sequence.intermediate_representation_content.r#while {
                if self
                    .unroll_intermediate_value(
                        &self.compute_node(
                            r#while,
                            &lockable_internals_write_guard.next_constants,
                        )?,
                    )?
                    .as_ref()
                    .as_ref()
                    .unwrap()
                    .as_bool()
                    .unwrap()
                {
                    lockable_internals_write_guard
                        .already_computed_values
                        .push(next);
                } else {
                    lockable_internals_write_guard.already_fully_computed = true;
                }
            } else {
                lockable_internals_write_guard
                    .already_computed_values
                    .push(next);
            }
        }
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
            let next_through = match &filter.throughs {
                Throughs::Array(node) => &**node,
                Throughs::Tuple {
                    nodes_indexes,
                    nodes,
                } => &nodes[nodes_indexes[next_input_value_index]],
            };
            let computed_next_through = self.compute_node(next_through, &next_constants)?;
            if (*self.unroll_intermediate_value(&computed_next_through)?)
                .clone()
                .unwrap()
                .as_bool()
                .unwrap()
            {
                lockable_internals_write_guard
                    .already_computed_values
                    .push(next_input_value);
            }
            lockable_internals_write_guard.already_processed_values_count += 1;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn compute_next_in_flatten(
        &self,
        flatten: &Flatten,
        lockable_internals_write_guard: &mut parking_lot::RwLockWriteGuard<
            FlattenLockableInternals,
        >,
    ) -> Result<bool> {
        loop {
            if let Some(current_element) = self.get_from_intermediate_value(
                &flatten.computed_flatten,
                lockable_internals_write_guard.next_element_index,
            )? {
                if let Some(current_element_element) = self.get_from_intermediate_value(
                    &current_element,
                    lockable_internals_write_guard.next_element_element_index,
                )? {
                    lockable_internals_write_guard.next_element_element_index += 1;
                    lockable_internals_write_guard
                        .already_computed_values
                        .push(current_element_element.clone());
                    return Ok(true);
                } else {
                    lockable_internals_write_guard.next_element_index += 1;
                    lockable_internals_write_guard.next_element_element_index = 0;
                }
            } else {
                return Ok(false);
            }
        }
    }

    fn get_by_key_from_intermediate_value(
        &self,
        intermediate_value_and_metadata: &Arc<IntermediateValueAndMetadata>,
        key: &String,
    ) -> Result<Option<Arc<IntermediateValueAndMetadata>>> {
        match &intermediate_value_and_metadata.intermediate_value {
            IntermediateValue::Value(value_arc) => match &**value_arc {
                Some(Value::Object(object)) => Ok(object.get(key).map(|value| {
                    IntermediateValueAndMetadata {
                        intermediate_value: IntermediateValue::Value(value.clone()),
                        type_kind: Value::type_kind(&value),
                    }
                    .into()
                })),
                unexpected_value => Err(anyhow!(
                    "expected tuple, sequence, map or filter, found {:#?}",
                    unexpected_value
                )),
            },
            IntermediateValue::Object(object) => Ok(object.get(key)),
            IntermediateValue::LazyValue(lazy_value) => {
                self.with_computed_lazy_value(lazy_value, |computed_lazy_value| {
                    self.get_by_key_from_intermediate_value(computed_lazy_value, key)
                })
            }
            unexpected_value => Err(anyhow!(
                "expected tuple, sequence, map or filter, found {:#?}",
                unexpected_value
            )),
        }
    }

    fn get_from_intermediate_value(
        &self,
        intermediate_value_and_metadata: &Arc<IntermediateValueAndMetadata>,
        index: usize,
    ) -> Result<Option<Arc<IntermediateValueAndMetadata>>> {
        match &intermediate_value_and_metadata.intermediate_value {
            IntermediateValue::Tuple(list) => Ok(list.get(index).cloned()),
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
                    while !lockable_internals_write_guard.already_fully_computed
                        && lockable_internals_write_guard.already_computed_values.len() <= index
                    {
                        self.compute_next_in_sequence(
                            sequence,
                            &mut lockable_internals_write_guard,
                        )?;
                    }
                    Ok(lockable_internals_write_guard
                        .already_computed_values
                        .get(index)
                        .cloned())
                }
            }
            IntermediateValue::Flatten(flatten) => {
                let lockable_internals_read_guard = flatten.lockable_internals.upgradable_read();
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
                            .compute_next_in_flatten(flatten, &mut lockable_internals_write_guard)?
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
                    index..index + 1,
                )?
                .inner
                .into_iter()
                .next()),
            IntermediateValue::Value(value_arc) => {
                let element_type = match &intermediate_value_and_metadata.type_kind {
                    TypeKind::Array(element_type) => *element_type.clone(),
                    TypeKind::Tuple(elements_types) => {
                        if let Some(result_type) = elements_types.get(index) {
                            result_type.clone()
                        } else {
                            return Ok(None);
                        }
                    }
                    _ => panic!(),
                };
                match &**value_arc {
                    Some(Value::Tuple(list)) => Ok(list.get(index).cloned().map(|element| {
                        IntermediateValueAndMetadata {
                            intermediate_value: IntermediateValue::Value(element),
                            type_kind: element_type.kind,
                        }
                        .into()
                    })),
                    unexpected_value => Err(anyhow!(
                        "expected tuple, sequence, map or filter, found {:#?}",
                        unexpected_value
                    )),
                }
            }
            IntermediateValue::LazyValue(lazy_value) => self
                .with_computed_lazy_value(lazy_value, |computed_lazy_value| {
                    self.get_from_intermediate_value(computed_lazy_value, index)
                }),
            unexpected_value => Err(anyhow!(
                "expected tuple, sequence, map or filter, found {:#?}",
                unexpected_value
            )),
        }
    }

    fn get_range_from_intermediate_value(
        &self,
        intermediate_value_and_metadata: &Arc<IntermediateValueAndMetadata>,
        range: Range<usize>,
    ) -> Result<Vector<IntermediateValueAndMetadata>> {
        match &intermediate_value_and_metadata.intermediate_value {
            IntermediateValue::Tuple(list) => Ok(Vector {
                inner: list
                    .iter()
                    .skip(range.start)
                    .take(range.end - range.start)
                    .cloned()
                    .collect(),
            }),
            IntermediateValue::Sequence(sequence) => {
                let lockable_internals_read_guard = sequence.lockable_internals.upgradable_read();
                if lockable_internals_read_guard.already_computed_values.len() >= range.end {
                    Ok(Vector {
                        inner: lockable_internals_read_guard
                            .already_computed_values
                            .iter()
                            .skip(range.start)
                            .take(range.end - range.start)
                            .cloned()
                            .collect(),
                    })
                } else {
                    let mut lockable_internals_write_guard =
                        parking_lot::RwLockUpgradableReadGuard::upgrade(
                            lockable_internals_read_guard,
                        );
                    while !lockable_internals_write_guard.already_fully_computed
                        && lockable_internals_write_guard.already_computed_values.len() < range.end
                    {
                        self.compute_next_in_sequence(
                            sequence,
                            &mut lockable_internals_write_guard,
                        )?;
                    }
                    Ok(Vector {
                        inner: lockable_internals_write_guard
                            .already_computed_values
                            .iter()
                            .skip(range.start)
                            .take(range.end - range.start)
                            .cloned()
                            .collect(),
                    })
                }
            }
            IntermediateValue::Flatten(flatten) => {
                let lockable_internals_read_guard = flatten.lockable_internals.upgradable_read();
                if lockable_internals_read_guard.already_computed_values.len() >= range.end {
                    Ok(Vector {
                        inner: lockable_internals_read_guard
                            .already_computed_values
                            .iter()
                            .skip(range.start)
                            .take(range.end - range.start)
                            .cloned()
                            .collect(),
                    })
                } else {
                    let mut lockable_internals_write_guard =
                        parking_lot::RwLockUpgradableReadGuard::upgrade(
                            lockable_internals_read_guard,
                        );
                    while lockable_internals_write_guard.already_computed_values.len() < range.end {
                        if !self
                            .compute_next_in_flatten(flatten, &mut lockable_internals_write_guard)?
                        {
                            break;
                        }
                    }
                    Ok(Vector {
                        inner: lockable_internals_write_guard
                            .already_computed_values
                            .iter()
                            .skip(range.start)
                            .take(range.end - range.start)
                            .cloned()
                            .collect(),
                    })
                }
            }
            IntermediateValue::Filter(filter) => {
                let lockable_internals_read_guard = filter.lockable_internals.upgradable_read();
                if lockable_internals_read_guard.already_computed_values.len() >= range.end {
                    Ok(Vector {
                        inner: lockable_internals_read_guard
                            .already_computed_values
                            .iter()
                            .skip(range.start)
                            .take(range.end - range.start)
                            .cloned()
                            .collect(),
                    })
                } else {
                    let mut lockable_internals_write_guard =
                        parking_lot::RwLockUpgradableReadGuard::upgrade(
                            lockable_internals_read_guard,
                        );
                    while lockable_internals_write_guard.already_computed_values.len() < range.end {
                        if !self
                            .compute_next_in_filter(filter, &mut lockable_internals_write_guard)?
                        {
                            break;
                        }
                    }
                    Ok(Vector {
                        inner: lockable_internals_write_guard
                            .already_computed_values
                            .iter()
                            .skip(range.start)
                            .take(range.end - range.start)
                            .cloned()
                            .collect(),
                    })
                }
            }
            IntermediateValue::Map(map) => {
                let computed_map_range =
                    self.get_range_from_intermediate_value(&map.computed_map, range.clone())?;
                let (already_taken_elements, elements_to_compute) = {
                    let mut lockable_internals_read_guard =
                        map.lockable_internals.upgradable_read();
                    let already_taken = lockable_internals_read_guard
                        .elements_taken_for_computation
                        .range(range.clone())
                        .map(|(key, value)| (*key, value.clone()))
                        .collect::<Vec<_>>();
                    let mut to_compute = Vec::new();
                    lockable_internals_read_guard.with_upgraded(|lockable_internals_write_guard| {
                        for element_index in range.start..computed_map_range.len() {
                            lockable_internals_write_guard
                                .elements_taken_for_computation
                                .entry(element_index)
                                .or_insert_with(|| {
                                    let element_value = Arc::new(RwLock::new(Arc::new(
                                        IntermediateValueAndMetadata::default(),
                                    )));
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
                                    == computed_map_element_index + range.start
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
                                    Some(match &map.throughs {
                                        Throughs::Array(node) => node,
                                        Throughs::Tuple {
                                            nodes_indexes,
                                            nodes,
                                        } => {
                                            &nodes[nodes_indexes
                                                [computed_map_element_index + range.start]]
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
                    *element_to_compute_value =
                        result.inner[element_to_compute_index - range.start].clone();
                }
                for (already_computed_value_index, already_computed_value) in
                    already_taken_elements.into_iter()
                {
                    result.inner[already_computed_value_index - range.start] =
                        already_computed_value.read().clone();
                }
                Ok(result)
            }
            IntermediateValue::LazyValue(lazy_value) => {
                self.with_computed_lazy_value(lazy_value, |computed_lazy_value| {
                    self.get_range_from_intermediate_value(computed_lazy_value, range.clone())
                })
            }
            IntermediateValue::Value(value_arc) => {
                let elements_types = match &intermediate_value_and_metadata.type_kind {
                    TypeKind::Array(element_type) => [*element_type.clone()].to_vec(),
                    TypeKind::Tuple(elements_types) => elements_types
                        .iter()
                        .skip(range.start)
                        .take(range.end - range.start)
                        .cloned()
                        .collect::<Vec<_>>(),
                    _ => panic!(),
                };
                match &**value_arc {
                    Some(Value::Tuple(list)) => Ok(Vector {
                        inner: list
                            .iter()
                            .skip(range.start)
                            .take(range.end - range.start)
                            .cloned()
                            .zip(elements_types)
                            .map(|(value, r#type)| {
                                IntermediateValueAndMetadata {
                                    intermediate_value: IntermediateValue::Value(value),
                                    type_kind: r#type.kind.clone(),
                                }
                                .into()
                            })
                            .collect(),
                    }),
                    unexpected_value => Err(anyhow!(
                        "expected tuple, sequence, map or filter, found {:#?}",
                        unexpected_value
                    )),
                }
            }
            unexpected_value => Err(anyhow!(
                "expected tuple, sequence, map or filter, found {:#?}",
                unexpected_value
            )),
        }
    }

    fn process_in_parallel<I, F, O>(
        &self,
        mut input: &[(usize, I)],
        function: &F,
        output: &Mutex<Vec<O>>,
    ) -> Result<()>
    where
        I: Send + Sync + Clone,
        F: Fn(I) -> Result<O> + Send + Sync,
        O: Send + Sync,
    {
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
                let (element_index, input_element) = input[0].clone();
                let node_result = function(input_element)?;
                output.lock()[element_index] = node_result;
                return Ok(());
            } else {
                let left_half = input.split_off(..input.len().div_ceil(2)).unwrap();
                for (element_index, input_element) in left_half.iter().cloned() {
                    let node_result = function(input_element)?;
                    output.lock()[element_index] = node_result;
                }
            }
        }
        if !input.is_empty() {
            let left_half = input.split_off(..input.len().div_ceil(2)).unwrap();
            let (left_half_result, right_half_result) = std::thread::scope(|scope| {
                let left_half_join_handle =
                    scope.spawn(|| self.process_in_parallel(left_half, function, output));
                let right_half_result = self.process_in_parallel(input, function, output);
                let left_half_result = left_half_join_handle
                    .join()
                    .map_err(|error| anyhow!("Thread panicked: {error:#?}"));
                *THREADS_LEFT_TO_SPAWN.lock() += 1;
                (left_half_result, right_half_result)
            });
            left_half_result??;
            right_half_result?;
        }
        Ok(())
    }

    fn unroll_intermediate_values<I>(
        &self,
        intermediate_values_iterator: I,
        intermediate_values_count: usize,
    ) -> Result<Vector<Option<Value>>>
    where
        I: Iterator<Item = Arc<IntermediateValueAndMetadata>>,
    {
        let mut result = vec![None; intermediate_values_count];
        let complex_elements = intermediate_values_iterator
            .enumerate()
            .filter_map(
                |(element_index, intermediate_value)| match &*intermediate_value {
                    IntermediateValueAndMetadata {
                        intermediate_value: IntermediateValue::Value(value),
                        type_kind: _,
                    } => {
                        result[element_index] = Some(value.clone());
                        None
                    }
                    _ => Some((element_index, intermediate_value)),
                },
            )
            .collect::<Vec<_>>();
        Ok(match complex_elements.len() {
            0 => Vector {
                inner: result.into_iter().map(Option::unwrap).collect(),
            },
            1 => {
                let (element_index, intermediate_value) =
                    complex_elements.into_iter().next().unwrap();
                result[element_index] = Some(self.unroll_intermediate_value(&intermediate_value)?);
                Vector {
                    inner: result.into_iter().map(Option::unwrap).collect(),
                }
            }
            2.. => {
                let result_mutex = Mutex::new(result);
                self.process_in_parallel(
                    &complex_elements,
                    &|intermediate_value| {
                        Ok(Some(self.unroll_intermediate_value(&intermediate_value)?))
                    },
                    &result_mutex,
                )?;
                return Ok(Vector::from_iter(
                    result_mutex.into_inner().into_iter().map(Option::unwrap),
                ));
            }
        })
    }

    fn unroll_intermediate_value(
        &self,
        intermediate_value: &Arc<IntermediateValueAndMetadata>,
    ) -> Result<Arc<Option<Value>>> {
        match &**intermediate_value {
            IntermediateValueAndMetadata {
                intermediate_value: IntermediateValue::Value(result),
                type_kind: _,
            } => Ok(result.clone()),
            IntermediateValueAndMetadata {
                intermediate_value: IntermediateValue::Tuple(intermediate_values_list),
                type_kind: _,
            } => Ok(Some(Value::Tuple(self.unroll_intermediate_values(
                intermediate_values_list.inner.iter().cloned(),
                intermediate_values_list.len(),
            )?))
            .into()),
            IntermediateValueAndMetadata {
                intermediate_value: IntermediateValue::Sequence(sequence),
                type_kind: _,
            } => {
                if sequence
                    .intermediate_representation_content
                    .r#while
                    .is_none()
                {
                    Err(anyhow!(
                        "can not possibly unroll sequence without `while` {intermediate_value:#?}"
                    ))
                } else {
                    let mut lockable_internals_write_guard = sequence.lockable_internals.write();
                    while !lockable_internals_write_guard.already_fully_computed {
                        self.compute_next_in_sequence(
                            sequence,
                            &mut lockable_internals_write_guard,
                        )?;
                    }
                    Ok(Some(Value::Tuple(
                        self.unroll_intermediate_values(
                            lockable_internals_write_guard
                                .already_computed_values
                                .iter()
                                .cloned(),
                            lockable_internals_write_guard.already_computed_values.len(),
                        )?,
                    ))
                    .into())
                }
            }
            IntermediateValueAndMetadata {
                intermediate_value: IntermediateValue::Object(object),
                type_kind: _,
            } => {
                let object_values = object.values().cloned().collect::<Vec<_>>();
                let object_values_len = object_values.len();
                Ok(Some(Value::Object(Object::from_iter(
                    object.keys().cloned().zip(
                        self.unroll_intermediate_values(
                            object_values.into_iter(),
                            object_values_len,
                        )?
                        .inner,
                    ),
                )))
                .into())
            }
            IntermediateValueAndMetadata {
                intermediate_value: IntermediateValue::Map(map),
                type_kind: _,
            } => {
                let computed_map_range =
                    self.get_range_from_intermediate_value(&map.computed_map, 0..usize::MAX)?;
                let computed_map_len = computed_map_range.len();
                Ok(Some(Value::Tuple({
                    let mut result = Vector::default();
                    for element in self
                        .compute_nodes(
                            computed_map_range.inner.into_iter().enumerate().map(
                                |(computed_map_element_index, computed_map_element)| {
                                    let mut through_constants = map.constants.clone();
                                    through_constants[map
                                        .intermediate_representation_content
                                        .map_constant_name_clustered_index] =
                                        Some(computed_map_element);
                                    (
                                        match &map.throughs {
                                            Throughs::Array(node) => Some(node),
                                            Throughs::Tuple {
                                                nodes_indexes,
                                                nodes,
                                            } => Some(
                                                &nodes[nodes_indexes[computed_map_element_index]],
                                            ),
                                        },
                                        Cow::Owned(through_constants),
                                    )
                                },
                            ),
                            computed_map_len,
                        )?
                        .inner
                    {
                        result.push(self.unroll_intermediate_value(&element)?);
                    }
                    result
                }))
                .into())
            }
            IntermediateValueAndMetadata {
                intermediate_value: IntermediateValue::LazyValue(lazy_value),
                type_kind: _,
            } => self.unroll_intermediate_value(&self.compute_lazy_value(lazy_value)?),
            IntermediateValueAndMetadata {
                intermediate_value: IntermediateValue::Flatten(flatten),
                type_kind: _,
            } => {
                {
                    let mut lockable_internals_write_guard = flatten.lockable_internals.write();
                    while self
                        .compute_next_in_flatten(flatten, &mut lockable_internals_write_guard)?
                    {
                    }
                }
                let lockable_internals_read_guard = flatten.lockable_internals.read();
                Ok(Arc::new(Some(Value::Tuple(
                    self.unroll_intermediate_values(
                        lockable_internals_read_guard
                            .already_computed_values
                            .iter()
                            .cloned(),
                        lockable_internals_read_guard.already_computed_values.len(),
                    )?,
                ))))
            }
            IntermediateValueAndMetadata {
                intermediate_value: IntermediateValue::Filter(filter),
                type_kind: _,
            } => {
                {
                    let mut lockable_internals_write_guard = filter.lockable_internals.write();
                    while self
                        .compute_next_in_filter(filter, &mut lockable_internals_write_guard)?
                    {
                    }
                }
                let lockable_internals_read_guard = filter.lockable_internals.read();
                Ok(Arc::new(Some(Value::Tuple(
                    self.unroll_intermediate_values(
                        lockable_internals_read_guard
                            .already_computed_values
                            .iter()
                            .cloned(),
                        lockable_internals_read_guard.already_computed_values.len(),
                    )?,
                ))))
            }
        }
    }

    fn compute_nodes<N>(
        &self,
        nodes_and_constants: N,
        nodes_count: usize,
    ) -> Result<Vector<IntermediateValueAndMetadata>>
    where
        N: Iterator<Item = (Option<&'a Arc<Node>>, Cow<'a, Constants>)>,
    {
        let initial_element = Arc::new(IntermediateValueAndMetadata::default());
        let mut result = vec![initial_element; nodes_count];
        let complex_elements = nodes_and_constants
            .enumerate()
            .filter_map(
                |(element_index, (node_option, constants))| match node_option {
                    Some(node) => match &node.content {
                        Content::Value(value) => {
                            result[element_index] = IntermediateValueAndMetadata {
                                intermediate_value: IntermediateValue::Value(unsafe {
                                    std::mem::transmute::<
                                        Arc<Option<intermediate_representation::Value>>,
                                        Arc<Option<Value>>,
                                    >(value.clone())
                                }),
                                type_kind: node.r#type.kind.clone(),
                            }
                            .into();
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
        Ok(Vector {
            inner: match complex_elements.len() {
                0 => result,
                1 => {
                    let (element_index, (node, constants)) =
                        complex_elements.into_iter().next().unwrap();
                    result[element_index] = self.compute_node(node, &constants)?;
                    result
                }
                2.. => {
                    let result_mutex = Mutex::new(result);
                    self.process_in_parallel(
                        &complex_elements,
                        &|(node, constants)| self.compute_node(node, &constants),
                        &result_mutex,
                    )?;
                    result_mutex.into_inner()
                }
            },
        })
    }

    fn compute_or_lazy(
        &self,
        node: &Arc<Node>,
        constants: &Constants,
    ) -> Result<Arc<IntermediateValueAndMetadata>> {
        Ok(match node.content {
            Content::Sequence(_)
            | Content::Fold { .. }
            | Content::EmbeddedFunctionCall { .. }
            | Content::UserFunctionCall { .. }
            | Content::FromAt { .. }
            | Content::Match { .. } => IntermediateValueAndMetadata {
                intermediate_value: IntermediateValue::LazyValue(LazyValue {
                    node: node.clone(),
                    constants: constants.clone(),
                    computed: Arc::new(Mutex::new(None)),
                }),
                type_kind: node.r#type.kind.clone(),
            }
            .into(),
            _ => self.compute_node(node, constants)?,
        })
    }

    fn compute_node(
        &self,
        node: &Node,
        constants: &Constants,
    ) -> Result<Arc<IntermediateValueAndMetadata>> {
        match &node.content {
            Content::Tuple(tuple) => {
                let mut result = Vector::default();
                for element in tuple {
                    result.push(self.compute_or_lazy(element, constants)?);
                }
                Ok(IntermediateValueAndMetadata {
                    intermediate_value: IntermediateValue::Tuple(result),
                    type_kind: concrete_or_else(&node.r#type.kind, || {
                        TypeKind::Tuple(
                            tuple
                                .iter()
                                .map(|tuple_element| tuple_element.r#type.clone())
                                .collect::<Vec<_>>()
                                .into(),
                        )
                    }),
                }
                .into())
            }
            Content::Scope {
                constants: scope_constants,
                compute,
            } => {
                let mut result_constants = constants.clone();
                for constant_definition in scope_constants {
                    result_constants[constant_definition.name_clustered_index] =
                        Some(self.compute_or_lazy(&constant_definition.node, constants)?);
                }
                self.compute_node(compute, &result_constants)
            }
            Content::Constant(constant_name_clustered_index) => {
                Ok(constants[*constant_name_clustered_index].clone().unwrap())
            }
            Content::EmbeddedFunctionCall {
                path_option,
                embedded_function_call,
            } => match embedded_function_call.embedded_function {
                EmbeddedFunction::Sleep => {
                    let duration = self
                        .unroll_intermediate_value(
                            &self.compute_node(&embedded_function_call.argument, constants)?,
                        )?
                        .as_ref()
                        .as_ref()
                        .unwrap()
                        .as_duration()
                        .unwrap();
                    std::thread::sleep(duration);
                    Ok(IntermediateValueAndMetadata {
                        intermediate_value: IntermediateValue::Value(
                            Some(Value::Duration { duration }).into(),
                        ),
                        type_kind: TypeKind::Number,
                    }
                    .into())
                }
                EmbeddedFunction::Sum => Ok(IntermediateValueAndMetadata {
                    intermediate_value: IntermediateValue::Value(
                        Some(Value::Number(
                            self.unroll_intermediate_value(
                                &self.compute_node(&embedded_function_call.argument, constants)?,
                            )?
                            .as_ref()
                            .as_ref()
                            .unwrap()
                            .as_tuple()
                            .unwrap()
                            .iter()
                            .fold(
                                Rational::ZERO,
                                |accumulator, current| {
                                    accumulator
                                        + current.as_ref().as_ref().unwrap().as_number().unwrap()
                                },
                            ),
                        ))
                        .into(),
                    ),
                    type_kind: TypeKind::Number,
                }
                .into()),
                EmbeddedFunction::SumTime => {
                    let (value, type_kind) = {
                        let argument_value = self.unroll_intermediate_value(
                            &self.compute_node(&embedded_function_call.argument, constants)?,
                        )?;
                        let argument_tuple = argument_value
                            .as_ref()
                            .as_ref()
                            .unwrap()
                            .as_tuple()
                            .unwrap();
                        if let Some(first_element) = &*argument_tuple.inner[0]
                            && matches!(first_element, Value::Time { time: _ })
                        {
                            (
                                Value::Time {
                                    time: first_element.as_time().unwrap()
                                        + argument_tuple.inner[1]
                                            .as_ref()
                                            .as_ref()
                                            .unwrap()
                                            .as_duration()
                                            .unwrap(),
                                },
                                TypeKind::Time,
                            )
                        } else {
                            (
                                Value::Duration {
                                    duration: argument_tuple.inner.iter().fold(
                                        std::time::Duration::ZERO,
                                        |accumulator, current| {
                                            accumulator
                                                + current
                                                    .as_ref()
                                                    .as_ref()
                                                    .unwrap()
                                                    .as_duration()
                                                    .unwrap()
                                        },
                                    ),
                                },
                                TypeKind::Duration,
                            )
                        }
                    };
                    Ok(IntermediateValueAndMetadata {
                        intermediate_value: IntermediateValue::Value(Some(value).into()),
                        type_kind,
                    }
                    .into())
                }
                EmbeddedFunction::Substract => {
                    let (value, type_kind) = {
                        let argument_value = self.unroll_intermediate_value(
                            &self.compute_node(&embedded_function_call.argument, constants)?,
                        )?;
                        let argument_tuple = argument_value
                            .as_ref()
                            .as_ref()
                            .unwrap()
                            .as_tuple()
                            .unwrap();
                        let minuend = argument_tuple.inner[0].as_ref().as_ref().unwrap();
                        let substrahend = argument_tuple.inner[1].as_ref().as_ref().unwrap();
                        match (minuend, substrahend) {
                            (Value::Number(minuend_number), Value::Number(substrahend_number)) => (
                                Value::Number(minuend_number - substrahend_number),
                                TypeKind::Number,
                            ),
                            (
                                Value::Duration {
                                    duration: minuend_duration,
                                },
                                Value::Duration {
                                    duration: substrahend_duration,
                                },
                            ) => (
                                Value::Duration {
                                    duration: minuend_duration
                                        .checked_sub(*substrahend_duration)
                                        .unwrap_or(std::time::Duration::ZERO),
                                },
                                TypeKind::Duration,
                            ),
                            (
                                Value::Time { time: minuend_time },
                                Value::Duration {
                                    duration: substrahend_duration,
                                },
                            ) => (
                                Value::Time {
                                    time: minuend_time
                                        .checked_sub(*substrahend_duration)
                                        .unwrap_or(std::time::SystemTime::UNIX_EPOCH),
                                },
                                TypeKind::Time,
                            ),
                            (
                                Value::Time { time: minuend_time },
                                Value::Time {
                                    time: substrahend_time,
                                },
                            ) => (
                                Value::Duration {
                                    duration: if minuend_time > substrahend_time {
                                        minuend_time.duration_since(*substrahend_time).unwrap()
                                    } else {
                                        std::time::Duration::ZERO
                                    },
                                },
                                TypeKind::Duration,
                            ),
                            _ => panic!(),
                        }
                    };
                    Ok(IntermediateValueAndMetadata {
                        intermediate_value: IntermediateValue::Value(Some(value).into()),
                        type_kind,
                    }
                    .into())
                }
                EmbeddedFunction::Mod => Ok(IntermediateValueAndMetadata {
                    intermediate_value: IntermediateValue::Value(
                        {
                            let unrolled_argument = self.unroll_intermediate_value(
                                &self.compute_node(&embedded_function_call.argument, constants)?,
                            )?;
                            let m = unrolled_argument
                                .as_ref()
                                .as_ref()
                                .unwrap()
                                .as_tuple()
                                .unwrap()
                                .get(1)
                                .unwrap()
                                .as_ref()
                                .as_ref()
                                .unwrap()
                                .as_number()
                                .unwrap();
                            if m == Rational::ZERO {
                                return Err(anyhow!("can not devide by zero"))
                                    .with_context(embedded_function_error_context(path_option))?;
                            } else {
                                let a = unrolled_argument
                                    .as_ref()
                                    .as_ref()
                                    .unwrap()
                                    .as_tuple()
                                    .unwrap()
                                    .get(0)
                                    .unwrap()
                                    .as_ref()
                                    .as_ref()
                                    .unwrap()
                                    .as_number()
                                    .unwrap();
                                let quotient = a.clone() / m.clone();
                                let floor = quotient.floor();
                                Some(Value::Number(a - floor * m))
                            }
                        }
                        .into(),
                    ),
                    type_kind: TypeKind::Number,
                }
                .into()),
                EmbeddedFunction::Divide => Ok(IntermediateValueAndMetadata {
                    intermediate_value: IntermediateValue::Value(
                        {
                            let unrolled_argument = self.unroll_intermediate_value(
                                &self.compute_node(&embedded_function_call.argument, constants)?,
                            )?;
                            let divider = unrolled_argument
                                .as_ref()
                                .as_ref()
                                .unwrap()
                                .as_tuple()
                                .unwrap()
                                .get(1)
                                .unwrap()
                                .as_ref()
                                .as_ref()
                                .unwrap()
                                .as_number()
                                .unwrap();
                            if divider == Rational::ZERO {
                                return Err(anyhow!("can not devide by zero"))
                                    .with_context(embedded_function_error_context(path_option))?;
                            } else {
                                let divided = unrolled_argument
                                    .as_ref()
                                    .as_ref()
                                    .unwrap()
                                    .as_tuple()
                                    .unwrap()
                                    .get(0)
                                    .unwrap()
                                    .as_ref()
                                    .as_ref()
                                    .unwrap()
                                    .as_number()
                                    .unwrap();
                                Some(Value::Number(divided / divider))
                            }
                        }
                        .into(),
                    ),
                    type_kind: TypeKind::Number,
                }
                .into()),
                EmbeddedFunction::Round => Ok(IntermediateValueAndMetadata {
                    intermediate_value: IntermediateValue::Value(
                        Some(Value::Number(
                            self.unroll_intermediate_value(
                                &self.compute_node(&embedded_function_call.argument, constants)?,
                            )?
                            .as_ref()
                            .as_ref()
                            .unwrap()
                            .as_number()
                            .unwrap()
                            .round()
                            .into(),
                        ))
                        .into(),
                    ),
                    type_kind: TypeKind::Number,
                }
                .into()),
                EmbeddedFunction::Exponentiate => Ok(IntermediateValueAndMetadata {
                    intermediate_value: IntermediateValue::Value(
                        {
                            let unrolled_argument = self.unroll_intermediate_value(
                                &self.compute_node(&embedded_function_call.argument, constants)?,
                            )?;
                            let argument_object = unrolled_argument
                                .as_ref()
                                .as_ref()
                                .unwrap()
                                .as_object()
                                .unwrap();
                            let base = argument_object
                                .inner
                                .get(&"base".to_string())
                                .unwrap()
                                .as_ref()
                                .as_ref()
                                .unwrap()
                                .as_number()
                                .unwrap();
                            let exponent = argument_object
                                .inner
                                .get(&"exponent".to_string())
                                .unwrap()
                                .as_ref()
                                .as_ref()
                                .unwrap()
                                .as_number()
                                .unwrap();
                            let precision = argument_object
                                .inner
                                .get(&"precision".to_string())
                                .unwrap()
                                .as_ref()
                                .as_ref()
                                .unwrap()
                                .as_number()
                                .unwrap()
                                .trunc()
                                .to_f64()
                                .value()
                                .max(1f64) as usize;
                            let max_result_denominator = argument_object
                                .inner
                                .get(&"max result denominator".to_string())
                                .unwrap()
                                .as_ref()
                                .as_ref()
                                .unwrap()
                                .as_number()
                                .unwrap()
                                .round()
                                .as_ubig()
                                .cloned()
                                .unwrap_or(dashu::ubig!(1));
                            let rational_result: Rational = (base
                                .to_float::<dashu::float::round::mode::Zero, 2>(precision)
                                .value()
                                .powf(&exponent.to_float(64).value()))
                            .try_into()
                            .unwrap();
                            Some(Value::Number(
                                rational_result.nearest(&max_result_denominator).value(),
                            ))
                        }
                        .into(),
                    ),
                    type_kind: TypeKind::Number,
                }
                .into()),
                EmbeddedFunction::Concat => {
                    let mut result_rope = ropey::Rope::default();
                    for appendix in self
                        .unroll_intermediate_value(
                            &self.compute_node(&embedded_function_call.argument, constants)?,
                        )?
                        .as_ref()
                        .as_ref()
                        .unwrap()
                        .as_tuple()
                        .unwrap()
                        .iter()
                    {
                        result_rope.append(
                            appendix
                                .as_ref()
                                .as_ref()
                                .unwrap()
                                .as_string()
                                .unwrap()
                                .clone(),
                        );
                    }
                    Ok(IntermediateValueAndMetadata {
                        intermediate_value: IntermediateValue::Value(
                            Some(Value::String(result_rope)).into(),
                        ),
                        type_kind: TypeKind::String,
                    }
                    .into())
                }
                EmbeddedFunction::IsSorted => Ok(IntermediateValueAndMetadata {
                    intermediate_value: IntermediateValue::Value(
                        Some(Value::Bool(
                            self.unroll_intermediate_value(
                                &self.compute_node(&embedded_function_call.argument, constants)?,
                            )?
                            .as_ref()
                            .as_ref()
                            .unwrap()
                            .as_tuple()
                            .unwrap()
                            .iter()
                            .is_sorted(),
                        ))
                        .into(),
                    ),
                    type_kind: TypeKind::Bool,
                }
                .into()),
                EmbeddedFunction::Sort => {
                    let mut argument_tuple = self
                        .unroll_intermediate_value(
                            &self.compute_node(&embedded_function_call.argument, constants)?,
                        )?
                        .as_ref()
                        .as_ref()
                        .unwrap()
                        .as_tuple()
                        .unwrap()
                        .clone();
                    argument_tuple.inner.sort();
                    let result_value = Some(Value::Tuple(argument_tuple));
                    let type_kind = Value::type_kind(&result_value);
                    Ok(IntermediateValueAndMetadata {
                        intermediate_value: IntermediateValue::Value(result_value.into()),
                        type_kind,
                    }
                    .into())
                }
                EmbeddedFunction::ReadBytesFromStandardInput => {
                    let computed_argument_unrolled = self.unroll_intermediate_value(
                        &self.compute_node(&embedded_function_call.argument, constants)?,
                    )?;
                    let result = if matches!(
                        computed_argument_unrolled.as_ref().as_ref().unwrap(),
                        Value::String(_)
                    ) {
                        // argument is "all"
                        let mut result = Vec::new();
                        std::io::stdin().read_to_end(&mut result)?;
                        result
                    } else {
                        let read_bytes_amount = (computed_argument_unrolled
                            .as_ref()
                            .as_ref()
                            .unwrap()
                            .as_number()
                            .unwrap()
                            .trunc()
                            .to_f64()
                            .value() as i64)
                            .max(0) as usize;
                        let mut result = vec![0; read_bytes_amount];
                        std::io::stdin()
                            .read_exact(&mut result)
                            .with_context(embedded_function_error_context(path_option))?;
                        result
                    };
                    Ok(IntermediateValueAndMetadata {
                        intermediate_value: IntermediateValue::Value(
                            Some(Value::Bytes {
                                bytes: result.into(),
                            })
                            .into(),
                        ),
                        type_kind: TypeKind::Bytes,
                    }
                    .into())
                }
                EmbeddedFunction::Parse => {
                    let computed_argument_unrolled = self.unroll_intermediate_value(
                        &self.compute_node(&embedded_function_call.argument, constants)?,
                    )?;
                    let argument_object = computed_argument_unrolled
                        .as_ref()
                        .as_ref()
                        .unwrap()
                        .as_object()
                        .unwrap();
                    let (format, source) = argument_object.inner.iter().next().unwrap();
                    let result_value: Option<Value> = match format.as_str() {
                        "YAML" => serde_saphyr::from_str(
                            &source
                                .as_ref()
                                .as_ref()
                                .unwrap()
                                .as_string()
                                .unwrap()
                                .to_string(),
                        )
                        .with_context(embedded_function_error_context(path_option))?,
                        "JSON" => serde_json::from_str(
                            &source
                                .as_ref()
                                .as_ref()
                                .unwrap()
                                .as_string()
                                .unwrap()
                                .to_string(),
                        )
                        .with_context(embedded_function_error_context(path_option))?,
                        "bincode" => {
                            let intermediate_representation_value: Option<
                                intermediate_representation::Value,
                            > = bincode::serde::decode_from_slice(
                                source.as_ref().as_ref().unwrap().as_bytes().unwrap(),
                                bincode::config::standard(),
                            )
                            .with_context(embedded_function_error_context(path_option))?
                            .0;
                            unsafe {
                                std::mem::transmute::<
                                    Option<intermediate_representation::Value>,
                                    Option<Value>,
                                >(intermediate_representation_value)
                            }
                        }
                        _ => panic!(),
                    };
                    let r#type = Value::type_kind(&result_value);
                    Ok(IntermediateValueAndMetadata {
                        intermediate_value: IntermediateValue::Value(result_value.into()),
                        type_kind: r#type,
                    }
                    .into())
                }
                EmbeddedFunction::StringifyTo => {
                    let computed_argument_unrolled = self.unroll_intermediate_value(
                        &self.compute_node(&embedded_function_call.argument, constants)?,
                    )?;
                    let argument_object = computed_argument_unrolled
                        .as_ref()
                        .as_ref()
                        .unwrap()
                        .as_object()
                        .unwrap();
                    let (format, source) = argument_object
                        .inner
                        .iter()
                        .map(|(key, value)| (key.as_str(), value.as_ref().as_ref()))
                        .next()
                        .unwrap();
                    let result_value = match format {
                        "YAML" => serde_saphyr::to_string(&source).unwrap(),
                        "JSON" => serde_json::to_string(&source).unwrap(),
                        _ => panic!(),
                    };
                    Ok(IntermediateValueAndMetadata {
                        intermediate_value: IntermediateValue::Value(
                            Some(Value::String(result_value.into())).into(),
                        ),
                        type_kind: TypeKind::String,
                    }
                    .into())
                }
                EmbeddedFunction::EncodeTo => {
                    let computed_argument_unrolled = self.unroll_intermediate_value(
                        &self.compute_node(&embedded_function_call.argument, constants)?,
                    )?;
                    let argument_object = computed_argument_unrolled
                        .as_ref()
                        .as_ref()
                        .unwrap()
                        .as_object()
                        .unwrap();
                    let (format, source) = argument_object
                        .inner
                        .iter()
                        .map(|(key, value)| (key.as_str(), value.as_ref().as_ref()))
                        .next()
                        .unwrap();
                    Ok(IntermediateValueAndMetadata {
                        intermediate_value: IntermediateValue::Value(
                            Some(Value::Bytes {
                                bytes: match format {
                                    "bincode" => bincode::serde::encode_to_vec(
                                        unsafe {
                                            std::mem::transmute::<
                                                Option<&Value>,
                                                Option<&intermediate_representation::Value>,
                                            >(source)
                                        },
                                        bincode::config::standard(),
                                    )
                                    .unwrap(),
                                    _ => panic!(),
                                }
                                .into(),
                            })
                            .into(),
                        ),
                        type_kind: TypeKind::Bytes,
                    }
                    .into())
                }
                EmbeddedFunction::Hash => {
                    let computed_argument_unrolled = self.unroll_intermediate_value(
                        &self.compute_node(&embedded_function_call.argument, constants)?,
                    )?;
                    let argument_object = computed_argument_unrolled
                        .as_ref()
                        .as_ref()
                        .unwrap()
                        .as_object()
                        .unwrap();
                    let algorithm_value =
                        argument_object.inner.get(&"algorithm".to_string()).unwrap();
                    let data_value = argument_object.inner.get(&"data".to_string()).unwrap();
                    let data = data_value.as_ref().as_ref().unwrap().as_bytes().unwrap();
                    let result_raw = match algorithm_value.as_ref().as_ref().unwrap() {
                        Value::String(algorithm_string) => {
                            match algorithm_string.to_string().as_str() {
                                "SHA224" => sha2::Sha224::digest(data).to_vec(),
                                "SHA256" => sha2::Sha256::digest(data).to_vec(),
                                "SHA384" => sha2::Sha384::digest(data).to_vec(),
                                "SHA512" => sha2::Sha512::digest(data).to_vec(),
                                "SHA512/224" => sha2::Sha512_224::digest(data).to_vec(),
                                "SHA512/256" => sha2::Sha512_256::digest(data).to_vec(),
                                "XXH3_64" => {
                                    xxhash_rust::xxh3::xxh3_64(data).to_be_bytes().to_vec()
                                }
                                "XXH3_128" => {
                                    xxhash_rust::xxh3::xxh3_128(data).to_be_bytes().to_vec()
                                }
                                "BLAKE3" => blake3::hash(data).as_bytes().to_vec(),
                                _ => panic!(),
                            }
                        }
                        Value::Object(algorithm_object) => {
                            let (algorithm, parameters) =
                                algorithm_object.inner.iter().next().unwrap();
                            match algorithm.as_str() {
                                "GxHash32" => gxhash::gxhash32(
                                    data,
                                    parameters
                                        .as_ref()
                                        .as_ref()
                                        .unwrap()
                                        .as_number()
                                        .unwrap()
                                        .trunc()
                                        .to_f64()
                                        .value() as i64,
                                )
                                .to_be_bytes()
                                .to_vec(),
                                "GxHash64" => gxhash::gxhash64(
                                    data,
                                    parameters
                                        .as_ref()
                                        .as_ref()
                                        .unwrap()
                                        .as_number()
                                        .unwrap()
                                        .trunc()
                                        .to_f64()
                                        .value() as i64,
                                )
                                .to_be_bytes()
                                .to_vec(),
                                "GxHash128" => gxhash::gxhash128(
                                    data,
                                    parameters
                                        .as_ref()
                                        .as_ref()
                                        .unwrap()
                                        .as_number()
                                        .unwrap()
                                        .trunc()
                                        .to_f64()
                                        .value() as i64,
                                )
                                .to_be_bytes()
                                .to_vec(),
                                "XXH32" => xxhash_rust::xxh32::xxh32(
                                    data,
                                    parameters
                                        .as_ref()
                                        .as_ref()
                                        .unwrap()
                                        .as_number()
                                        .unwrap()
                                        .trunc()
                                        .to_f64()
                                        .value() as u32,
                                )
                                .to_be_bytes()
                                .to_vec(),
                                "XXH64" => xxhash_rust::xxh64::xxh64(
                                    data,
                                    parameters
                                        .as_ref()
                                        .as_ref()
                                        .unwrap()
                                        .as_number()
                                        .unwrap()
                                        .trunc()
                                        .to_f64()
                                        .value() as u64,
                                )
                                .to_be_bytes()
                                .to_vec(),
                                "BLAKE3 with key" => {
                                    let provided_key =
                                        parameters.as_ref().as_ref().unwrap().as_bytes().unwrap();
                                    let mut key = [0u8; 32];
                                    let bytes_amount_to_take = provided_key.len().min(32);
                                    key[..bytes_amount_to_take]
                                        .copy_from_slice(&provided_key[..bytes_amount_to_take]);
                                    blake3::keyed_hash(&key, data).as_bytes().to_vec()
                                }
                                "BLAKE3 with size" => {
                                    let size = parameters
                                        .as_ref()
                                        .as_ref()
                                        .unwrap()
                                        .as_number()
                                        .unwrap()
                                        .trunc()
                                        .to_f64()
                                        .value()
                                        as usize;
                                    let mut result = vec![0u8; size];
                                    blake3::Hasher::new()
                                        .update(data)
                                        .finalize_xof()
                                        .fill(&mut result);
                                    result.to_vec()
                                }
                                "BLAKE3 derive key" => {
                                    let context = parameters
                                        .as_ref()
                                        .as_ref()
                                        .unwrap()
                                        .as_string()
                                        .unwrap()
                                        .to_string();
                                    blake3::derive_key(&context, data).to_vec()
                                }
                                _ => panic!(),
                            }
                        }
                        _ => panic!(),
                    };
                    Ok(IntermediateValueAndMetadata {
                        intermediate_value: IntermediateValue::Value(
                            Some(Value::Bytes {
                                bytes: result_raw.into(),
                            })
                            .into(),
                        ),
                        type_kind: TypeKind::Bytes,
                    }
                    .into())
                }
                EmbeddedFunction::KeyValuePairs => {
                    let unrolled_argument = self.unroll_intermediate_value(
                        &self.compute_node(&embedded_function_call.argument, constants)?,
                    )?;
                    Ok(IntermediateValueAndMetadata {
                        intermediate_value: IntermediateValue::Value(
                            Some(Value::Tuple({
                                let mut result = Vector::default();
                                for (key, value) in unrolled_argument
                                    .as_ref()
                                    .as_ref()
                                    .unwrap()
                                    .as_object()
                                    .unwrap()
                                    .iter()
                                {
                                    result.push(
                                        Some(Value::Tuple(Vector::from_iter([
                                            Some(Value::String(ropey::Rope::from_str(key))).into(),
                                            value.clone(),
                                        ])))
                                        .into(),
                                    );
                                }
                                result
                            }))
                            .into(),
                        ),
                        type_kind: concrete_or_else(&node.r#type.kind, || {
                            TypeKind::Tuple(
                                unrolled_argument
                                    .as_ref()
                                    .as_ref()
                                    .unwrap()
                                    .as_object()
                                    .unwrap()
                                    .values()
                                    .map(|value| {
                                        node.r#type.with_kind(TypeKind::Tuple(
                                            vec![
                                                node.r#type.with_kind(TypeKind::String),
                                                Value::type_kind(value).into(),
                                            ]
                                            .into(),
                                        ))
                                    })
                                    .collect::<Vec<_>>()
                                    .into(),
                            )
                        }),
                    }
                    .into())
                }
                EmbeddedFunction::CompileRegex => {
                    let computed_argument_unrolled = self.unroll_intermediate_value(
                        &self.compute_node(&embedded_function_call.argument, constants)?,
                    )?;
                    let computed_regex_value =
                        computed_argument_unrolled.as_ref().as_ref().unwrap();
                    let computed_regex_string = {
                        if let Value::String(computed_regex_string) = computed_regex_value {
                            computed_regex_string.to_string()
                        } else {
                            self.build_regex(computed_regex_value.as_tuple().unwrap())
                        }
                    };
                    let compiled_regex = Regex::new(&computed_regex_string)?;
                    Ok(IntermediateValueAndMetadata {
                        intermediate_value: IntermediateValue::Value(
                            Some(Value::Runtime(
                                Runtime::CompiledRegex(Identified::new(compiled_regex)).into(),
                            ))
                            .into(),
                        ),
                        type_kind: TypeKind::Embedded(Embedded::CompiledRegex),
                    }
                    .into())
                }
                EmbeddedFunction::HttpClient => {
                    let computed_argument_unrolled = self.unroll_intermediate_value(
                        &self.compute_node(&embedded_function_call.argument, constants)?,
                    )?;
                    let mut agent_config_builder = ureq::config::Config::builder();
                    for (agent_config_builder_key, agent_config_builder_value) in
                        computed_argument_unrolled
                            .as_ref()
                            .as_ref()
                            .unwrap()
                            .as_tuple()
                            .unwrap()
                            .inner
                            .iter()
                            .map(|object_value| {
                                let object =
                                    object_value.as_ref().as_ref().unwrap().as_object().unwrap();
                                let (key, value) = object.inner.iter().next().unwrap();
                                (key.as_str(), value.clone())
                            })
                    {
                        match agent_config_builder_key {
                            "error on bad status code" => {
                                agent_config_builder = agent_config_builder.http_status_as_error(
                                    agent_config_builder_value
                                        .as_ref()
                                        .as_ref()
                                        .unwrap()
                                        .as_bool()
                                        .unwrap(),
                                );
                            }
                            "HTTPS only" => {
                                agent_config_builder = agent_config_builder.https_only(
                                    agent_config_builder_value
                                        .as_ref()
                                        .as_ref()
                                        .unwrap()
                                        .as_bool()
                                        .unwrap(),
                                );
                            }
                            "no delay" => {
                                agent_config_builder = agent_config_builder.no_delay(
                                    agent_config_builder_value
                                        .as_ref()
                                        .as_ref()
                                        .unwrap()
                                        .as_bool()
                                        .unwrap(),
                                );
                            }
                            "max redirects" => {
                                agent_config_builder = agent_config_builder.max_redirects(
                                    agent_config_builder_value
                                        .as_ref()
                                        .as_ref()
                                        .unwrap()
                                        .as_number()
                                        .unwrap()
                                        .trunc()
                                        .to_f64()
                                        .value()
                                        .max(0f64)
                                        .min(u32::MAX as f64)
                                        as u32,
                                );
                            }
                            "max response header size" => {
                                agent_config_builder = agent_config_builder
                                    .max_response_header_size(
                                        agent_config_builder_value
                                            .as_ref()
                                            .as_ref()
                                            .unwrap()
                                            .as_number()
                                            .unwrap()
                                            .trunc()
                                            .to_f64()
                                            .value()
                                            .max(0f64)
                                            as usize,
                                    );
                            }
                            "input buffer size" => {
                                agent_config_builder = agent_config_builder.input_buffer_size(
                                    agent_config_builder_value
                                        .as_ref()
                                        .as_ref()
                                        .unwrap()
                                        .as_number()
                                        .unwrap()
                                        .trunc()
                                        .to_f64()
                                        .value()
                                        .max(0f64) as usize,
                                );
                            }
                            "max idle connections" => {
                                agent_config_builder = agent_config_builder.max_idle_connections(
                                    agent_config_builder_value
                                        .as_ref()
                                        .as_ref()
                                        .unwrap()
                                        .as_number()
                                        .unwrap()
                                        .trunc()
                                        .to_f64()
                                        .value()
                                        .max(0f64) as usize,
                                );
                            }
                            "max idle connections per host" => {
                                agent_config_builder = agent_config_builder
                                    .max_idle_connections_per_host(
                                        agent_config_builder_value
                                            .as_ref()
                                            .as_ref()
                                            .unwrap()
                                            .as_number()
                                            .unwrap()
                                            .trunc()
                                            .to_f64()
                                            .value()
                                            .max(0f64)
                                            as usize,
                                    );
                            }
                            "max idle age" => {
                                agent_config_builder = agent_config_builder.max_idle_age(
                                    agent_config_builder_value
                                        .as_ref()
                                        .as_ref()
                                        .unwrap()
                                        .as_duration()
                                        .unwrap(),
                                );
                            }
                            "allow non-standard methods" => {
                                agent_config_builder = agent_config_builder
                                    .allow_non_standard_methods(
                                        agent_config_builder_value
                                            .as_ref()
                                            .as_ref()
                                            .unwrap()
                                            .as_bool()
                                            .unwrap(),
                                    )
                            }
                            "timeout global" => {
                                agent_config_builder = agent_config_builder.timeout_global(Some(
                                    agent_config_builder_value
                                        .as_ref()
                                        .as_ref()
                                        .unwrap()
                                        .as_duration()
                                        .unwrap(),
                                ));
                            }
                            "timeout per call" => {
                                agent_config_builder = agent_config_builder.timeout_per_call(Some(
                                    agent_config_builder_value
                                        .as_ref()
                                        .as_ref()
                                        .unwrap()
                                        .as_duration()
                                        .unwrap(),
                                ));
                            }
                            "timeout for resolve" => {
                                agent_config_builder = agent_config_builder.timeout_resolve(Some(
                                    agent_config_builder_value
                                        .as_ref()
                                        .as_ref()
                                        .unwrap()
                                        .as_duration()
                                        .unwrap(),
                                ));
                            }
                            "timeout for connect" => {
                                agent_config_builder = agent_config_builder.timeout_connect(Some(
                                    agent_config_builder_value
                                        .as_ref()
                                        .as_ref()
                                        .unwrap()
                                        .as_duration()
                                        .unwrap(),
                                ));
                            }
                            "timeout for send request" => {
                                agent_config_builder =
                                    agent_config_builder.timeout_send_request(Some(
                                        agent_config_builder_value
                                            .as_ref()
                                            .as_ref()
                                            .unwrap()
                                            .as_duration()
                                            .unwrap(),
                                    ));
                            }
                            "timeout for await 100" => {
                                agent_config_builder =
                                    agent_config_builder.timeout_await_100(Some(
                                        agent_config_builder_value
                                            .as_ref()
                                            .as_ref()
                                            .unwrap()
                                            .as_duration()
                                            .unwrap(),
                                    ));
                            }
                            "timeout for send body" => {
                                agent_config_builder =
                                    agent_config_builder.timeout_send_body(Some(
                                        agent_config_builder_value
                                            .as_ref()
                                            .as_ref()
                                            .unwrap()
                                            .as_duration()
                                            .unwrap(),
                                    ));
                            }
                            "timeout for receive body" => {
                                agent_config_builder =
                                    agent_config_builder.timeout_recv_body(Some(
                                        agent_config_builder_value
                                            .as_ref()
                                            .as_ref()
                                            .unwrap()
                                            .as_duration()
                                            .unwrap(),
                                    ));
                            }
                            "IP family" => {
                                agent_config_builder = agent_config_builder.ip_family(
                                    match agent_config_builder_value
                                        .as_ref()
                                        .as_ref()
                                        .unwrap()
                                        .as_string()
                                        .unwrap()
                                        .to_string()
                                        .as_str()
                                    {
                                        "any" => IpFamily::Any,
                                        "IPv4" => IpFamily::Ipv4Only,
                                        "IPv6" => IpFamily::Ipv6Only,
                                        _ => panic!(),
                                    },
                                );
                            }
                            "redirect authorization headers" => {
                                agent_config_builder = agent_config_builder.redirect_auth_headers(
                                    match agent_config_builder_value
                                        .as_ref()
                                        .as_ref()
                                        .unwrap()
                                        .as_string()
                                        .unwrap()
                                        .to_string()
                                        .as_str()
                                    {
                                        "never" => RedirectAuthHeaders::Never,
                                        "same host" => RedirectAuthHeaders::SameHost,
                                        _ => panic!(),
                                    },
                                );
                            }
                            "user agent" => {
                                match agent_config_builder_value.as_ref().as_ref().unwrap() {
                                    Value::Bool(_) => {
                                        agent_config_builder =
                                            agent_config_builder.user_agent(AutoHeaderValue::None);
                                    }
                                    Value::String(header_value) => {
                                        agent_config_builder = agent_config_builder.user_agent(
                                            AutoHeaderValue::Provided(
                                                header_value.to_string().into(),
                                            ),
                                        );
                                    }
                                    _ => panic!(),
                                }
                            }
                            "accept" => {
                                match agent_config_builder_value.as_ref().as_ref().unwrap() {
                                    Value::Bool(_) => {
                                        agent_config_builder =
                                            agent_config_builder.accept(AutoHeaderValue::None);
                                    }
                                    Value::String(header_value) => {
                                        agent_config_builder =
                                            agent_config_builder.accept(AutoHeaderValue::Provided(
                                                header_value.to_string().into(),
                                            ));
                                    }
                                    _ => panic!(),
                                }
                            }
                            "accept encoding" => {
                                match agent_config_builder_value.as_ref().as_ref().unwrap() {
                                    Value::Bool(_) => {
                                        agent_config_builder = agent_config_builder
                                            .accept_encoding(AutoHeaderValue::None);
                                    }
                                    Value::String(header_value) => {
                                        agent_config_builder = agent_config_builder
                                            .accept_encoding(AutoHeaderValue::Provided(
                                                header_value.to_string().into(),
                                            ));
                                    }
                                    _ => panic!(),
                                }
                            }
                            "proxy" => {
                                match agent_config_builder_value.as_ref().as_ref().unwrap() {
                                    Value::Bool(_) => {
                                        agent_config_builder = agent_config_builder.proxy(None);
                                    }
                                    Value::String(proxy_string_value) => {
                                        agent_config_builder = agent_config_builder.proxy(Some(
                                            ureq::Proxy::new(&proxy_string_value.to_string())
                                                .with_context(embedded_function_error_context(
                                                    path_option,
                                                ))?,
                                        ));
                                    }
                                    Value::Object(proxy_builder_builder_key_value_pairs_object) => {
                                        let mut proxy_builder = ureq::Proxy::builder(
                                            match proxy_builder_builder_key_value_pairs_object
                                                .inner
                                                .get(&"protocol".to_string())
                                                .unwrap()
                                                .as_ref()
                                                .as_ref()
                                                .unwrap()
                                                .as_string()
                                                .unwrap()
                                                .to_string()
                                                .as_str()
                                            {
                                                "HTTP" => ProxyProtocol::Http,
                                                "HTTPS" => ProxyProtocol::Https,
                                                "SOCKS4" => ProxyProtocol::Socks4,
                                                "SOCKS4A" => ProxyProtocol::Socks4A,
                                                "SOCKS5" => ProxyProtocol::Socks5,
                                                "SOCKS5h" => ProxyProtocol::Socks5h,
                                                _ => panic!(),
                                            },
                                        );
                                        for (proxy_builder_key, proxy_builder_value) in
                                            proxy_builder_builder_key_value_pairs_object
                                                .inner
                                                .get(&"configuration".to_string())
                                                .unwrap()
                                                .as_ref()
                                                .as_ref()
                                                .unwrap()
                                                .as_tuple()
                                                .unwrap()
                                                .inner
                                                .iter()
                                                .map(|object_value| {
                                                    let object = object_value
                                                        .as_ref()
                                                        .as_ref()
                                                        .unwrap()
                                                        .as_object()
                                                        .unwrap();
                                                    let (key, value) =
                                                        object.inner.iter().next().unwrap();
                                                    (key.as_str(), value.clone())
                                                })
                                        {
                                            match proxy_builder_key {
                                                "host" => {
                                                    proxy_builder = proxy_builder.host(
                                                        &proxy_builder_value
                                                            .as_ref()
                                                            .as_ref()
                                                            .unwrap()
                                                            .as_string()
                                                            .unwrap()
                                                            .to_string(),
                                                    );
                                                }
                                                "port" => {
                                                    proxy_builder = proxy_builder.port(
                                                        proxy_builder_value
                                                            .as_ref()
                                                            .as_ref()
                                                            .unwrap()
                                                            .as_number()
                                                            .unwrap()
                                                            .trunc()
                                                            .to_f64()
                                                            .value()
                                                            .max(0f64)
                                                            .min(u16::MAX as f64)
                                                            as u16,
                                                    );
                                                }
                                                "user name" => {
                                                    proxy_builder = proxy_builder.username(
                                                        &proxy_builder_value
                                                            .as_ref()
                                                            .as_ref()
                                                            .unwrap()
                                                            .as_string()
                                                            .unwrap()
                                                            .to_string(),
                                                    );
                                                }
                                                "password" => {
                                                    proxy_builder = proxy_builder.password(
                                                        &proxy_builder_value
                                                            .as_ref()
                                                            .as_ref()
                                                            .unwrap()
                                                            .as_string()
                                                            .unwrap()
                                                            .to_string(),
                                                    );
                                                }
                                                "resolve locally" => {
                                                    proxy_builder = proxy_builder.resolve_target(
                                                        proxy_builder_value
                                                            .as_ref()
                                                            .as_ref()
                                                            .unwrap()
                                                            .as_bool()
                                                            .unwrap(),
                                                    );
                                                }
                                                _ => panic!(),
                                            }
                                        }
                                    }
                                    _ => panic!(),
                                }
                            }
                            "TLS" => {
                                let mut tls_config_builder = ureq::tls::TlsConfig::builder();
                                for (tls_config_builder_key, tls_config_builder_value) in
                                    agent_config_builder_value
                                        .as_ref()
                                        .as_ref()
                                        .unwrap()
                                        .as_tuple()
                                        .unwrap()
                                        .inner
                                        .iter()
                                        .map(|object_value| {
                                            let object = object_value
                                                .as_ref()
                                                .as_ref()
                                                .unwrap()
                                                .as_object()
                                                .unwrap();
                                            let (key, value) = object.inner.iter().next().unwrap();
                                            (key.as_str(), value.clone())
                                        })
                                {
                                    match tls_config_builder_key {
                                        "use SNI" => {
                                            tls_config_builder = tls_config_builder.use_sni(
                                                tls_config_builder_value
                                                    .as_ref()
                                                    .as_ref()
                                                    .unwrap()
                                                    .as_bool()
                                                    .unwrap(),
                                            );
                                        }
                                        "client certificate" => {
                                            let client_certificate_config =
                                                tls_config_builder_value
                                                    .as_ref()
                                                    .as_ref()
                                                    .unwrap()
                                                    .as_object()
                                                    .unwrap();
                                            let mut chain = Vec::new();
                                            let chain_config = client_certificate_config
                                                .get(&"chain".to_string())
                                                .unwrap();
                                            for (
                                                certificate_config_key,
                                                certificate_config_value,
                                            ) in chain_config
                                                .as_ref()
                                                .as_ref()
                                                .unwrap()
                                                .as_tuple()
                                                .unwrap()
                                                .inner
                                                .iter()
                                                .map(|object_value| {
                                                    let object = object_value
                                                        .as_ref()
                                                        .as_ref()
                                                        .unwrap()
                                                        .as_object()
                                                        .unwrap();
                                                    let (key, value) =
                                                        object.inner.iter().next().unwrap();
                                                    (key.as_str(), value.clone())
                                                })
                                            {
                                                chain.push(match certificate_config_key {
                                                    "DER" => ureq::tls::Certificate::from_der(
                                                        certificate_config_value
                                                            .as_ref()
                                                            .as_ref()
                                                            .unwrap()
                                                            .as_bytes()
                                                            .unwrap(),
                                                    )
                                                    .to_owned(),
                                                    "PEM" => ureq::tls::Certificate::from_pem(
                                                        certificate_config_value
                                                            .as_ref()
                                                            .as_ref()
                                                            .unwrap()
                                                            .as_string()
                                                            .unwrap()
                                                            .to_string()
                                                            .as_bytes(),
                                                    )
                                                    .with_context(embedded_function_error_context(
                                                        path_option,
                                                    ))?
                                                    .to_owned(),
                                                    _ => panic!(),
                                                });
                                            }
                                            let private_key = ureq::tls::PrivateKey::from_pem(
                                                client_certificate_config
                                                    .get(&"private key PEM".to_string())
                                                    .unwrap()
                                                    .as_ref()
                                                    .as_ref()
                                                    .unwrap()
                                                    .as_string()
                                                    .unwrap()
                                                    .to_string()
                                                    .as_bytes(),
                                            )
                                            .with_context(embedded_function_error_context(
                                                path_option,
                                            ))?;
                                            tls_config_builder = tls_config_builder.client_cert(
                                                Some(ureq::tls::ClientCert::new_with_certs(
                                                    &chain,
                                                    private_key,
                                                )),
                                            );
                                        }
                                        "root certificates" => {
                                            match tls_config_builder_value
                                                .as_ref()
                                                .as_ref()
                                                .unwrap()
                                            {
                                                Value::String(root_certificates_config_string) => {
                                                    match root_certificates_config_string
                                                        .to_string()
                                                        .as_str()
                                                    {
                                                        "platform verifier" => {
                                                            tls_config_builder = tls_config_builder.root_certs(ureq::tls::RootCerts::PlatformVerifier);
                                                        }
                                                        "WebPKI" => {
                                                            tls_config_builder = tls_config_builder
                                                                .root_certs(
                                                                    ureq::tls::RootCerts::WebPki,
                                                                );
                                                        }
                                                        _ => panic!(),
                                                    }
                                                }
                                                Value::Tuple(root_certificates_config_tuple) => {
                                                    let mut chain = Vec::new();
                                                    for (
                                                        certificate_config_key,
                                                        certificate_config_value,
                                                    ) in root_certificates_config_tuple
                                                        .inner
                                                        .iter()
                                                        .map(|object_value| {
                                                            let object = object_value
                                                                .as_ref()
                                                                .as_ref()
                                                                .unwrap()
                                                                .as_object()
                                                                .unwrap();
                                                            let (key, value) =
                                                                object.inner.iter().next().unwrap();
                                                            (key.as_str(), value.clone())
                                                        })
                                                    {
                                                        chain.push(match certificate_config_key {
                                                            "DER" => {
                                                                ureq::tls::Certificate::from_der(
                                                                    certificate_config_value
                                                                        .as_ref()
                                                                        .as_ref()
                                                                        .unwrap()
                                                                        .as_bytes()
                                                                        .unwrap(),
                                                                )
                                                                .to_owned()
                                                            }
                                                            "PEM" => {
                                                                ureq::tls::Certificate::from_pem(
                                                                    certificate_config_value
                                                                        .as_ref()
                                                                        .as_ref()
                                                                        .unwrap()
                                                                        .as_string()
                                                                        .unwrap()
                                                                        .to_string()
                                                                        .as_bytes(),
                                                                )
                                                                .with_context(
                                                                    embedded_function_error_context(
                                                                        path_option,
                                                                    ),
                                                                )?
                                                                .to_owned()
                                                            }
                                                            _ => panic!(),
                                                        });
                                                    }
                                                    tls_config_builder = tls_config_builder
                                                        .root_certs(
                                                            ureq::tls::RootCerts::Specific(
                                                                chain.into(),
                                                            ),
                                                        );
                                                }
                                                _ => panic!(),
                                            }
                                        }
                                        _ => panic!(),
                                    }
                                }
                                agent_config_builder =
                                    agent_config_builder.tls_config(tls_config_builder.build());
                            }
                            _ => panic!(),
                        }
                    }
                    Ok(IntermediateValueAndMetadata {
                        intermediate_value: IntermediateValue::Value(
                            Some(Value::Runtime(
                                Runtime::HttpClient(Identified::new(ureq::Agent::new_with_config(
                                    agent_config_builder.build(),
                                )))
                                .into(),
                            ))
                            .into(),
                        ),
                        type_kind: TypeKind::Embedded(Embedded::HttpClient),
                    }
                    .into())
                }
                EmbeddedFunction::SendRequest => {
                    let computed_argument_unrolled = self.unroll_intermediate_value(
                        &self.compute_node(&embedded_function_call.argument, constants)?,
                    )?;
                    let argument_object = computed_argument_unrolled
                        .as_ref()
                        .as_ref()
                        .unwrap()
                        .as_object()
                        .unwrap();
                    let agent_value = argument_object.get(&"client".to_string()).unwrap();
                    let agent = agent_value
                        .as_ref()
                        .as_ref()
                        .unwrap()
                        .as_runtime()
                        .unwrap()
                        .as_http_client()
                        .unwrap();
                    let request_arguments = argument_object.get(&"request".to_string()).unwrap();
                    let request_arguments_object = request_arguments
                        .as_ref()
                        .as_ref()
                        .unwrap()
                        .as_object()
                        .unwrap();
                    let url = request_arguments_object
                        .get(&"URL".to_string())
                        .unwrap()
                        .as_ref()
                        .as_ref()
                        .unwrap()
                        .as_string()
                        .unwrap()
                        .to_string();
                    let method = request_arguments_object
                        .get(&"method".to_string())
                        .unwrap()
                        .as_ref()
                        .as_ref()
                        .unwrap()
                        .as_string()
                        .unwrap()
                        .to_string();
                    let mut request_builder = match method.as_str() {
                        "GET" => agent.get(url).force_send_body(),
                        "POST" => agent.post(url),
                        "PUT" => agent.put(url),
                        "DELETE" => agent.delete(url).force_send_body(),
                        "HEAD" => agent.head(url).force_send_body(),
                        "OPTIONS" => agent.options(url).force_send_body(),
                        "CONNECT" => agent.connect(url).force_send_body(),
                        "PATCH" => agent.patch(url),
                        "TRACE" => agent.trace(url).force_send_body(),
                        _ => panic!(),
                    };
                    let parameters_value = request_arguments_object
                        .get(&"parameters".to_string())
                        .unwrap();
                    for (parameter_key, parameter_value) in parameters_value
                        .as_ref()
                        .as_ref()
                        .unwrap()
                        .as_tuple()
                        .unwrap()
                        .inner
                        .iter()
                        .map(|object_value| {
                            let object =
                                object_value.as_ref().as_ref().unwrap().as_object().unwrap();
                            let (key, value) = object.inner.iter().next().unwrap();
                            (key.as_str(), value)
                        })
                    {
                        match parameter_key {
                            "content type" => {
                                let content_value_object = parameter_value
                                    .as_ref()
                                    .as_ref()
                                    .unwrap()
                                    .as_object()
                                    .unwrap();
                                let r#type = content_value_object
                                    .get(&"type".to_string())
                                    .unwrap()
                                    .as_ref()
                                    .as_ref()
                                    .unwrap()
                                    .as_string()
                                    .unwrap()
                                    .to_string();
                                let subtype = content_value_object
                                    .get(&"subtype".to_string())
                                    .unwrap()
                                    .as_ref()
                                    .as_ref()
                                    .unwrap()
                                    .as_string()
                                    .unwrap()
                                    .to_string();
                                let parameters_arguments =
                                    content_value_object.get(&"parameters".to_string()).unwrap();
                                let parameters_arguments_object = parameters_arguments
                                    .as_ref()
                                    .as_ref()
                                    .unwrap()
                                    .as_object()
                                    .unwrap();
                                let mut content_type_string = format!("{type}/{subtype}");
                                if !parameters_arguments_object.is_empty() {
                                    content_type_string.push(';');
                                    for (parameter_key, parameter_value) in
                                        parameters_arguments_object.inner.iter().map(
                                            |(key, value)| {
                                                (
                                                    &**key,
                                                    value
                                                        .as_ref()
                                                        .as_ref()
                                                        .unwrap()
                                                        .as_string()
                                                        .unwrap()
                                                        .to_string(),
                                                )
                                            },
                                        )
                                    {
                                        content_type_string.push(' ');
                                        content_type_string.push_str(parameter_key);
                                        content_type_string.push_str(&parameter_value);
                                    }
                                }
                                request_builder = request_builder.content_type(content_type_string);
                            }
                            "headers" => {
                                let content_value_tuple = parameter_value
                                    .as_ref()
                                    .as_ref()
                                    .unwrap()
                                    .as_tuple()
                                    .unwrap();
                                for (header_key, header_value) in
                                    content_value_tuple.inner.iter().map(|key_value_value| {
                                        let key_value_tuple = key_value_value
                                            .as_ref()
                                            .as_ref()
                                            .unwrap()
                                            .as_tuple()
                                            .unwrap();
                                        let key = key_value_tuple.inner[0]
                                            .as_ref()
                                            .as_ref()
                                            .unwrap()
                                            .as_string()
                                            .unwrap()
                                            .to_string();
                                        let value = key_value_tuple.inner[1]
                                            .as_ref()
                                            .as_ref()
                                            .unwrap()
                                            .as_string()
                                            .unwrap()
                                            .to_string();
                                        (key, value)
                                    })
                                {
                                    request_builder =
                                        request_builder.header(header_key, header_value);
                                }
                            }
                            "query" => {
                                let content_value_object = parameter_value
                                    .as_ref()
                                    .as_ref()
                                    .unwrap()
                                    .as_object()
                                    .unwrap();
                                let query_key_value_pairs_iterator =
                                    content_value_object.inner.iter().map(|(key, value)| {
                                        (
                                            &**key,
                                            value
                                                .as_ref()
                                                .as_ref()
                                                .unwrap()
                                                .as_string()
                                                .unwrap()
                                                .to_string(),
                                        )
                                    });
                                request_builder =
                                    request_builder.query_pairs(query_key_value_pairs_iterator);
                            }
                            "version" => {
                                let version = match parameter_value
                                    .as_ref()
                                    .as_ref()
                                    .unwrap()
                                    .as_string()
                                    .unwrap()
                                    .to_string()
                                    .as_str()
                                {
                                    "HTTP/0.9" => ureq::http::version::Version::HTTP_09,
                                    "HTTP/1.0" => ureq::http::version::Version::HTTP_10,
                                    "HTTP/1.1" => ureq::http::version::Version::HTTP_11,
                                    "HTTP/2.0" => ureq::http::version::Version::HTTP_2,
                                    "HTTP/3.0" => ureq::http::version::Version::HTTP_3,
                                    _ => panic!(),
                                };
                                request_builder = request_builder.version(version);
                            }
                            _ => panic!(),
                        }
                    }
                    let body_value_option =
                        request_arguments_object.get(&"body".to_string()).unwrap();
                    let response = if let Some(body_value) = body_value_option.as_ref().as_ref() {
                        let body_object = body_value.as_object().unwrap();
                        let (body_key, body_value) = body_object.inner.iter().next().unwrap();
                        match body_key.as_str() {
                            "data" => request_builder
                                .send(&**body_value.as_ref().as_ref().unwrap().as_bytes().unwrap()),
                            "form" => request_builder.send_form(
                                body_value
                                    .as_ref()
                                    .as_ref()
                                    .unwrap()
                                    .as_object()
                                    .unwrap()
                                    .inner
                                    .iter()
                                    .map(|(key, value)| {
                                        (
                                            key.as_str(),
                                            value
                                                .as_ref()
                                                .as_ref()
                                                .unwrap()
                                                .as_string()
                                                .unwrap()
                                                .to_string(),
                                        )
                                    }),
                            ),
                            _ => panic!(),
                        }
                    } else {
                        request_builder.send_empty()
                    }
                    .with_context(embedded_function_error_context(path_option))?;
                    Ok(IntermediateValueAndMetadata {
                        intermediate_value: IntermediateValue::Value(
                            Some(Value::Object(Object::from_iter([
                                (
                                    "status".to_string().into(),
                                    response
                                        .status()
                                        .canonical_reason()
                                        .map(|status_code_string| {
                                            Value::String(status_code_string.into())
                                        })
                                        .into(),
                                ),
                                (
                                    "is informational".to_string().into(),
                                    Some(Value::Bool(response.status().is_informational())).into(),
                                ),
                                (
                                    "is success".to_string().into(),
                                    Some(Value::Bool(response.status().is_success())).into(),
                                ),
                                (
                                    "is redirection".to_string().into(),
                                    Some(Value::Bool(response.status().is_redirection())).into(),
                                ),
                                (
                                    "is client error".to_string().into(),
                                    Some(Value::Bool(response.status().is_client_error())).into(),
                                ),
                                (
                                    "is server error".to_string().into(),
                                    Some(Value::Bool(response.status().is_server_error())).into(),
                                ),
                                (
                                    "version".to_string().into(),
                                    Some(Value::String(
                                        match response.version() {
                                            ureq::http::version::Version::HTTP_09 => "HTTP/0.9",
                                            ureq::http::version::Version::HTTP_10 => "HTTP/1.0",
                                            ureq::http::version::Version::HTTP_11 => "HTTP/1.1",
                                            ureq::http::version::Version::HTTP_2 => "HTTP/2",
                                            ureq::http::version::Version::HTTP_3 => "HTTP/3",
                                            _ => panic!(),
                                        }
                                        .into(),
                                    ))
                                    .into(),
                                ),
                                (
                                    "headers".to_string().into(),
                                    Some(Value::Object(Object::from_iter(
                                        response.headers().iter().map(
                                            |(header_key, header_value)| {
                                                (
                                                    header_key.to_string().into(),
                                                    Some(Value::String(
                                                        header_value.to_str().unwrap().into(),
                                                    ))
                                                    .into(),
                                                )
                                            },
                                        ),
                                    )))
                                    .into(),
                                ),
                                (
                                    "body".to_string().into(),
                                    Some(Value::Bytes {
                                        bytes: {
                                            let mut body_buffer = Vec::new();
                                            response
                                                .into_body()
                                                .into_reader()
                                                .read_to_end(&mut body_buffer)
                                                .with_context(embedded_function_error_context(
                                                    path_option,
                                                ))?;
                                            body_buffer.into()
                                        },
                                    })
                                    .into(),
                                ),
                            ])))
                            .into(),
                        ),
                        type_kind: node.r#type.kind.clone(),
                    }
                    .into())
                }
                EmbeddedFunction::MatchRegex => {
                    let computed_argument_unrolled = self.unroll_intermediate_value(
                        &self.compute_node(&embedded_function_call.argument, constants)?,
                    )?;
                    let computed_string = computed_argument_unrolled
                        .as_ref()
                        .as_ref()
                        .unwrap()
                        .as_object()
                        .unwrap()
                        .get(&"string".to_string())
                        .unwrap()
                        .as_ref()
                        .as_ref()
                        .unwrap()
                        .as_string()
                        .unwrap()
                        .to_string();
                    let regex_value = computed_argument_unrolled
                        .as_ref()
                        .as_ref()
                        .unwrap()
                        .as_object()
                        .unwrap()
                        .get(&"regex".to_string())
                        .unwrap();
                    let compiled_regex = regex_value
                        .as_ref()
                        .as_ref()
                        .unwrap()
                        .as_runtime()
                        .unwrap()
                        .as_compiled_regex()
                        .unwrap();
                    if let Some(captures) = compiled_regex.captures(&computed_string) {
                        let result_value = Some(Value::Object(Object::from_iter([
                            (
                                "groups".to_string().into(),
                                Some(Value::Object(Object::from_iter(
                                    compiled_regex.capture_names().flatten().filter_map(|name| {
                                        captures.name(name).map(|match_obj| {
                                            (
                                                name.to_string().into(),
                                                Some({
                                                    let matched = match_obj.as_str();
                                                    if let Ok(matched_as_number) =
                                                        dashu::Rational::from_str(matched)
                                                    {
                                                        Value::Number(matched_as_number)
                                                    } else {
                                                        Value::String(matched.into())
                                                    }
                                                })
                                                .into(),
                                            )
                                        })
                                    }),
                                )))
                                .into(),
                            ),
                            (
                                "start".to_string().into(),
                                Some(Value::Number(captures.get(0).unwrap().start().into())).into(),
                            ),
                            (
                                "end".to_string().into(),
                                Some(Value::Number(captures.get(0).unwrap().end().into())).into(),
                            ),
                        ])));
                        let r#type = Value::type_kind(&result_value);
                        Ok(IntermediateValueAndMetadata {
                            intermediate_value: IntermediateValue::Value(result_value.into()),
                            type_kind: r#type,
                        }
                        .into())
                    } else {
                        Ok(IntermediateValueAndMetadata {
                            intermediate_value: IntermediateValue::Value(None.into()),
                            type_kind: TypeKind::Null,
                        }
                        .into())
                    }
                }
                EmbeddedFunction::BufferizedWriter => {
                    let computed_argument_unrolled = self.unroll_intermediate_value(
                        &self.compute_node(&embedded_function_call.argument, constants)?,
                    )?;
                    let computed_path = computed_argument_unrolled
                        .as_ref()
                        .as_ref()
                        .unwrap()
                        .as_object()
                        .unwrap()
                        .get(&"path".to_string())
                        .unwrap()
                        .as_ref()
                        .as_ref()
                        .unwrap()
                        .as_string()
                        .unwrap()
                        .to_string();
                    let mut open_options = std::fs::OpenOptions::new();
                    match computed_argument_unrolled
                        .as_ref()
                        .as_ref()
                        .unwrap()
                        .as_object()
                        .unwrap()
                        .get(&"mode".to_string())
                        .unwrap()
                        .as_ref()
                        .as_ref()
                        .unwrap()
                        .as_string()
                        .unwrap()
                        .to_string()
                        .as_str()
                    {
                        "append" => {
                            open_options.append(true);
                        }
                        "overwrite" => {
                            open_options.write(true);
                        }
                        "truncate" => {
                            open_options.write(true).truncate(true);
                        }
                        "create or append" => {
                            open_options.append(true).create(true);
                        }
                        "create or overwrite" => {
                            open_options.write(true).create(true);
                        }
                        "create" => {
                            open_options.write(true).create_new(true);
                        }
                        _ => {}
                    };
                    let file = open_options
                        .open(&computed_path)
                        .with_context(embedded_function_error_context(path_option))?;
                    Ok(IntermediateValueAndMetadata {
                        intermediate_value: IntermediateValue::Value(
                            Some(Value::Runtime(
                                Runtime::BufferizedWriter(Identified::new(
                                    BufWriter::new(file).into(),
                                ))
                                .into(),
                            ))
                            .into(),
                        ),
                        type_kind: TypeKind::Embedded(Embedded::BufferizedWriter),
                    }
                    .into())
                }
                EmbeddedFunction::BufferizedReader => {
                    let computed_argument_unrolled = self.unroll_intermediate_value(
                        &self.compute_node(&embedded_function_call.argument, constants)?,
                    )?;
                    let computed_path = computed_argument_unrolled
                        .as_ref()
                        .as_ref()
                        .unwrap()
                        .as_string()
                        .unwrap()
                        .to_string();
                    let file = std::fs::OpenOptions::new()
                        .read(true)
                        .open(&computed_path)
                        .with_context(embedded_function_error_context(path_option))?;
                    Ok(IntermediateValueAndMetadata {
                        intermediate_value: IntermediateValue::Value(
                            Some(Value::Runtime(
                                Runtime::BufferizedReader(crate::value::Identified::new(
                                    BufReader::new(file).into(),
                                ))
                                .into(),
                            ))
                            .into(),
                        ),
                        type_kind: TypeKind::Embedded(Embedded::BufferizedReader),
                    }
                    .into())
                }
                EmbeddedFunction::WriteBytes => {
                    let computed_argument_unrolled = self.unroll_intermediate_value(
                        &self.compute_node(&embedded_function_call.argument, constants)?,
                    )?;
                    let computed_target = computed_argument_unrolled
                        .as_ref()
                        .as_ref()
                        .unwrap()
                        .as_object()
                        .unwrap()
                        .get(&"target".to_string())
                        .unwrap();
                    let computed_bufferized_writer = computed_target
                        .as_ref()
                        .as_ref()
                        .unwrap()
                        .as_runtime()
                        .unwrap()
                        .as_bufferized_writer()
                        .unwrap();
                    let computed_content_value = computed_argument_unrolled
                        .as_ref()
                        .as_ref()
                        .unwrap()
                        .as_object()
                        .unwrap()
                        .get(&"content".to_string())
                        .unwrap();
                    let computed_content = computed_content_value
                        .as_ref()
                        .as_ref()
                        .unwrap()
                        .as_bytes()
                        .unwrap();
                    let bytes_written = computed_bufferized_writer
                        .write()
                        .write(computed_content)
                        .with_context(embedded_function_error_context(path_option))?;
                    Ok(IntermediateValueAndMetadata {
                        intermediate_value: IntermediateValue::Value(
                            Some(Value::Number(bytes_written.into())).into(),
                        ),
                        type_kind: TypeKind::Number,
                    }
                    .into())
                }
                EmbeddedFunction::ReadBytes => {
                    let computed_argument_unrolled = self.unroll_intermediate_value(
                        &self.compute_node(&embedded_function_call.argument, constants)?,
                    )?;
                    let computed_source = computed_argument_unrolled
                        .as_ref()
                        .as_ref()
                        .unwrap()
                        .as_object()
                        .unwrap()
                        .get(&"source".to_string())
                        .unwrap();
                    let computed_bufferized_reader = computed_source
                        .as_ref()
                        .as_ref()
                        .unwrap()
                        .as_runtime()
                        .unwrap()
                        .as_bufferized_reader()
                        .unwrap();
                    let computed_amount = computed_argument_unrolled
                        .as_ref()
                        .as_ref()
                        .unwrap()
                        .as_object()
                        .unwrap()
                        .get(&"amount".to_string())
                        .unwrap()
                        .as_ref()
                        .as_ref()
                        .unwrap()
                        .as_number()
                        .unwrap()
                        .trunc()
                        .to_f64()
                        .value()
                        .floor()
                        .max(0f64) as usize;
                    if computed_amount == 0 {
                        Ok(IntermediateValueAndMetadata {
                            intermediate_value: IntermediateValue::Value(
                                Some(Value::Bytes {
                                    bytes: Bytes::new(),
                                })
                                .into(),
                            ),
                            type_kind: TypeKind::Bytes,
                        }
                        .into())
                    } else {
                        let mut result_raw = vec![0u8; computed_amount];
                        computed_bufferized_reader
                            .write()
                            .read(&mut result_raw)
                            .with_context(embedded_function_error_context(path_option))?;
                        Ok(IntermediateValueAndMetadata {
                            intermediate_value: IntermediateValue::Value(
                                Some(Value::Bytes {
                                    bytes: result_raw.into(),
                                })
                                .into(),
                            ),
                            type_kind: TypeKind::Bytes,
                        }
                        .into())
                    }
                }
                EmbeddedFunction::StringFromBytes => {
                    Ok(IntermediateValueAndMetadata {
                        intermediate_value: IntermediateValue::Value(
                            Some(Value::String(
                                String::from_utf8(
                                    self.unroll_intermediate_value(&self.compute_node(
                                        &embedded_function_call.argument,
                                        constants,
                                    )?)?
                                    .as_ref()
                                    .as_ref()
                                    .unwrap()
                                    .as_bytes()
                                    .unwrap()
                                    .to_vec(),
                                )
                                .with_context(embedded_function_error_context(path_option))?
                                .into(),
                            ))
                            .into(),
                        ),
                        type_kind: TypeKind::String,
                    }
                    .into())
                }
                EmbeddedFunction::BytesFromString => {
                    Ok(IntermediateValueAndMetadata {
                        intermediate_value: IntermediateValue::Value(
                            Some(Value::Bytes {
                                bytes: Bytes::from(
                                    self.unroll_intermediate_value(&self.compute_node(
                                        &embedded_function_call.argument,
                                        constants,
                                    )?)?
                                    .as_ref()
                                    .as_ref()
                                    .unwrap()
                                    .as_string()
                                    .unwrap()
                                    .bytes()
                                    .collect::<Vec<_>>(),
                                ),
                            })
                            .into(),
                        ),
                        type_kind: TypeKind::String,
                    }
                    .into())
                }
                EmbeddedFunction::RemoveFile => {
                    std::fs::remove_file(std::path::PathBuf::from(
                        self.unroll_intermediate_value(
                            &self.compute_node(&embedded_function_call.argument, constants)?,
                        )?
                        .as_ref()
                        .as_ref()
                        .unwrap()
                        .as_string()
                        .unwrap()
                        .to_string(),
                    ))
                    .with_context(embedded_function_error_context(path_option))?;
                    Ok(IntermediateValueAndMetadata {
                        intermediate_value: IntermediateValue::Value(None.into()),
                        type_kind: TypeKind::Null,
                    }
                    .into())
                }
                EmbeddedFunction::Glob => {
                    let mut result = Vector::default();
                    for entry in glob(
                        self.unroll_intermediate_value(
                            &self.compute_node(&embedded_function_call.argument, constants)?,
                        )?
                        .as_ref()
                        .as_ref()
                        .unwrap()
                        .as_string()
                        .unwrap()
                        .to_string()
                        .as_str(),
                    )
                    .with_context(embedded_function_error_context(path_option))?
                    {
                        result.push(
                            Some(Value::String(
                                entry
                                    .with_context(embedded_function_error_context(path_option))?
                                    .to_str()
                                    .with_context(embedded_function_error_context(path_option))?
                                    .to_owned()
                                    .into(),
                            ))
                            .into(),
                        );
                    }
                    Ok(IntermediateValueAndMetadata {
                        intermediate_value: IntermediateValue::Value(
                            Some(Value::Tuple(result)).into(),
                        ),
                        type_kind: EMBEDDED_FUNCTIONS
                            [embedded_function_call.embedded_function as usize]
                            [embedded_function_call.variant_index as usize]
                            .r#return
                            .r#type
                            .clone()
                            .unwrap(),
                    }
                    .into())
                }
                EmbeddedFunction::FileMetadata => {
                    let metadata = std::fs::metadata(
                        self.unroll_intermediate_value(
                            &self.compute_node(&embedded_function_call.argument, constants)?,
                        )?
                        .as_ref()
                        .as_ref()
                        .unwrap()
                        .as_string()
                        .unwrap()
                        .to_string(),
                    )
                    .with_context(embedded_function_error_context(path_option))?;
                    Ok(IntermediateValueAndMetadata {
                        intermediate_value: IntermediateValue::Value(
                            Some(Value::Object(Object::from_iter([
                                (
                                    "is directory".to_string().into(),
                                    Some(Value::Bool(metadata.is_dir())).into(),
                                ),
                                (
                                    "is file".to_string().into(),
                                    Some(Value::Bool(metadata.is_file())).into(),
                                ),
                                (
                                    "is symlink".to_string().into(),
                                    Some(Value::Bool(metadata.is_symlink())).into(),
                                ),
                                (
                                    "size".to_string().into(),
                                    Some(Value::Number(metadata.len().into())).into(),
                                ),
                                (
                                    "is read-only".to_string().into(),
                                    Some(Value::Bool(metadata.permissions().readonly())).into(),
                                ),
                                (
                                    "when modified".to_string().into(),
                                    Some(Value::Time {
                                        time: metadata.modified().with_context(
                                            embedded_function_error_context(path_option),
                                        )?,
                                    })
                                    .into(),
                                ),
                                (
                                    "when accessed".to_string().into(),
                                    Some(Value::Time {
                                        time: metadata.accessed().with_context(
                                            embedded_function_error_context(path_option),
                                        )?,
                                    })
                                    .into(),
                                ),
                                (
                                    "when created".to_string().into(),
                                    Some(Value::Time {
                                        time: metadata.created().with_context(
                                            embedded_function_error_context(path_option),
                                        )?,
                                    })
                                    .into(),
                                ),
                            ])))
                            .into(),
                        ),
                        type_kind: EMBEDDED_FUNCTIONS
                            [embedded_function_call.embedded_function as usize]
                            [embedded_function_call.variant_index as usize]
                            .r#return
                            .r#type
                            .clone()
                            .unwrap(),
                    }
                    .into())
                }
                EmbeddedFunction::Run => {
                    let computed_argument_unrolled = self.unroll_intermediate_value(
                        &self.compute_node(&embedded_function_call.argument, constants)?,
                    )?;
                    let command_name = computed_argument_unrolled
                        .as_ref()
                        .as_ref()
                        .unwrap()
                        .as_object()
                        .unwrap()
                        .get(&"command".to_string())
                        .unwrap()
                        .as_ref()
                        .as_ref()
                        .unwrap()
                        .as_string()
                        .unwrap()
                        .to_string();
                    let arguments = computed_argument_unrolled
                        .as_ref()
                        .as_ref()
                        .unwrap()
                        .as_object()
                        .unwrap()
                        .get(&"arguments".to_string())
                        .unwrap()
                        .as_ref()
                        .as_ref()
                        .unwrap()
                        .as_tuple()
                        .unwrap()
                        .clone();
                    let result = std::process::Command::new(command_name)
                        .args(arguments.iter().map(|argument_value| {
                            argument_value
                                .as_ref()
                                .as_ref()
                                .unwrap()
                                .as_string()
                                .unwrap()
                                .to_string()
                        }))
                        .output()
                        .with_context(embedded_function_error_context(path_option))?;
                    Ok(IntermediateValueAndMetadata {
                        intermediate_value: IntermediateValue::Value(
                            Some(Value::Object(Object::from_iter([
                                (
                                    "exit code".to_string().into(),
                                    Some(Value::Number(result.status.code().unwrap().into()))
                                        .into(),
                                ),
                                (
                                    "standard output".to_string().into(),
                                    Some(Value::Bytes {
                                        bytes: result.stdout.into(),
                                    })
                                    .into(),
                                ),
                                (
                                    "standard error".to_string().into(),
                                    Some(Value::Bytes {
                                        bytes: result.stderr.into(),
                                    })
                                    .into(),
                                ),
                            ])))
                            .into(),
                        ),
                        type_kind: node.r#type.kind.clone(),
                    }
                    .into())
                }
            },
            Content::UserFunctionCall {
                arguments,
                body,
                arguments_is_pure,
            } => {
                let mut result_constants = constants.clone();
                for (constant_name_clustered_index, computed_constant) in arguments
                    .iter()
                    .map(|constant_definition| constant_definition.name_clustered_index)
                    .zip(
                        self.compute_nodes(
                            arguments.iter().map(|constant_definition| {
                                (Some(&constant_definition.node), Cow::Borrowed(constants))
                            }),
                            arguments.len(),
                        )?
                        .inner,
                    )
                {
                    result_constants[constant_name_clustered_index] = Some(computed_constant);
                }
                let user_function = &self.intermediate_representation.user_functions[*body];
                if self.computer_config.user_functions_caching && *arguments_is_pure {
                    let function_call_identifier = {
                        let mut hasher = gxhash::GxHasher::default();
                        for constant_name_clustered_index in
                            &user_function.external_constants_name_clustered_indices
                        {
                            result_constants[*constant_name_clustered_index]
                                .as_ref()
                                .unwrap()
                                .hash(&mut hasher);
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
                default,
            } => {
                let mut result = self.compute_node(from, constants)?;
                for path_segment in value_path_segments {
                    match path_segment {
                        ValuePathSegment::ArrayIndex(array_index) => {
                            if let Some(new_result) = &mut self
                                .get_from_intermediate_value(&result, *array_index)
                                .with_context(|| {
                                    format!(
                                        "can not get element by index {array_index} from\n{}",
                                        serde_saphyr::to_string(&result.type_kind).unwrap()
                                    )
                                })?
                            {
                                result = std::mem::take(new_result);
                            } else {
                                return self.compute_node(default, constants);
                            }
                        }
                        ValuePathSegment::ObjectKey(object_key) => {
                            if let Some(new_result) = &mut self
                                .get_by_key_from_intermediate_value(&result, object_key)
                                .with_context(|| {
                                    format!(
                                        "can not get value by key {object_key:?} from\n{}",
                                        serde_saphyr::to_string(&result.type_kind).unwrap()
                                    )
                                })?
                            {
                                result = std::mem::take(new_result);
                            } else {
                                return self.compute_node(default, constants);
                            }
                        }
                        ValuePathSegment::ArrayRange {
                            from: range_from,
                            to: range_to,
                        } => {
                            let from_number = match &**range_from {
                                RangeBound::Static(Some(range_from)) => *range_from,
                                RangeBound::Static(None) => 0,
                                RangeBound::Dynamic(from_node) => {
                                    self.unroll_intermediate_value(
                                        &self.compute_node(from_node, constants)?,
                                    )?
                                    .as_ref()
                                    .as_ref()
                                    .unwrap()
                                    .as_number()
                                    .unwrap()
                                    .trunc()
                                    .to_f64()
                                    .value()
                                    .max(0f64) as usize
                                }
                            };
                            let to_number = match &**range_to {
                                RangeBound::Static(Some(range_to)) => *range_to,
                                RangeBound::Static(None) => usize::MAX,
                                RangeBound::Dynamic(to_node) => self
                                    .unroll_intermediate_value(
                                        &self.compute_node(to_node, constants)?,
                                    )?
                                    .as_ref()
                                    .as_ref()
                                    .unwrap()
                                    .as_number()
                                    .unwrap()
                                    .trunc()
                                    .to_f64()
                                    .value()
                                    .min(usize::MAX as f64)
                                    as usize,
                            };
                            if result.type_kind.contains(&TypeKind::GenericLiteralString) {
                                let unrolled_result = self.unroll_intermediate_value(&result)?;
                                let string_value: ropey::Rope = if to_number == usize::MAX {
                                    unrolled_result
                                        .as_ref()
                                        .as_ref()
                                        .unwrap()
                                        .as_string()
                                        .unwrap()
                                        .get_slice(from_number..)
                                } else {
                                    unrolled_result
                                        .as_ref()
                                        .as_ref()
                                        .unwrap()
                                        .as_string()
                                        .unwrap()
                                        .get_slice(from_number..to_number)
                                }
                                .unwrap_or_else(|| "".into())
                                .into();
                                result = IntermediateValueAndMetadata {
                                    intermediate_value: IntermediateValue::Value(
                                        Some(Value::String(string_value.clone())).into(),
                                    ),
                                    type_kind: TypeKind::LiteralString(string_value),
                                }
                                .into();
                            } else {
                                let result_elements = self
                                    .get_range_from_intermediate_value(
                                        &result,
                                        from_number..to_number,
                                    )
                                    .with_context(|| {
                                        format!(
                                            "can not get slice by range from\n{}",
                                            serde_saphyr::to_string(&result.type_kind).unwrap()
                                        )
                                    })?
                                    .inner
                                    .into_iter()
                                    .collect::<Vec<_>>();
                                let r#type = TypeKind::Tuple(
                                    result_elements
                                        .iter()
                                        .map(|element| element.type_kind.clone().into())
                                        .collect::<Vec<_>>()
                                        .into(),
                                );
                                result = IntermediateValueAndMetadata {
                                    intermediate_value: IntermediateValue::Tuple(Vector {
                                        inner: result_elements,
                                    }),
                                    type_kind: r#type,
                                }
                                .into();
                            }
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
                    let computed_match_unrolled_lazy_cell =
                        LazyCell::new(|| self.unroll_intermediate_value(&computed_match));
                    for case in cases {
                        match &case.condition {
                            Condition::Type(expected_type) => {
                                if expected_type.kind.contains(&computed_match.type_kind) {
                                    return self.compute_node(&case.node, &case_constants);
                                }
                            }
                            Condition::Value(expected_value_node) => {
                                let computed_expected_value = self.unroll_intermediate_value(
                                    &self.compute_node(expected_value_node, constants)?,
                                )?;
                                if &computed_expected_value == {
                                    match &*computed_match_unrolled_lazy_cell {
                                        Ok(result) => result,
                                        Err(error) => return Err(anyhow!(error.to_string())),
                                    }
                                } {
                                    return self.compute_node(&case.node, &case_constants);
                                }
                            }
                        }
                    }
                    panic!(
                        "no case from {cases:#?} matches computed value \
                         {computed_match_unrolled_lazy_cell:#?} of type {:#?}",
                        computed_match.type_kind
                    );
                }
            }
            Content::Map(intermediate_representation_content) => {
                let computed_map =
                    self.compute_node(&intermediate_representation_content.map, constants)?;
                let mut throughs_option = None;
                for (map_concrete_type, throughs) in intermediate_representation_content
                    .map_concrete_type_and_throughs
                    .iter()
                {
                    if map_concrete_type.kind.contains(&computed_map.type_kind) {
                        throughs_option = Some(throughs.clone());
                    }
                }
                let r#type = concrete_or_else(&node.r#type.kind, || {
                    match throughs_option.as_ref().unwrap() {
                        Throughs::Array(through_node) => {
                            TypeKind::Array(Box::new(through_node.r#type.clone()))
                        }
                        Throughs::Tuple {
                            nodes_indexes: _,
                            nodes,
                        } => TypeKind::Tuple(
                            nodes
                                .iter()
                                .map(|node| node.r#type.clone())
                                .collect::<Vec<_>>()
                                .into(),
                        ),
                    }
                });
                Ok(IntermediateValueAndMetadata {
                    intermediate_value: IntermediateValue::Map(Map {
                        intermediate_representation_content: intermediate_representation_content
                            .clone(),
                        computed_map,
                        throughs: throughs_option.unwrap(),
                        constants: constants.clone(),
                        lockable_internals: Arc::new(RwLock::new(MapLockableInternals {
                            elements_taken_for_computation: BTreeMap::new(),
                        })),
                    }),
                    type_kind: r#type,
                }
                .into())
            }
            Content::Flatten(flatten) => Ok(IntermediateValueAndMetadata {
                intermediate_value: IntermediateValue::Flatten(Flatten {
                    computed_flatten: self.compute_node(flatten, constants)?,
                    lockable_internals: Arc::new(RwLock::new(FlattenLockableInternals::default())),
                }),
                type_kind: node.r#type.kind.clone(),
            }
            .into()),
            Content::Filter(intermediate_representation_content) => {
                let computed_filter = Box::new(
                    self.compute_node(&intermediate_representation_content.filter, constants)?,
                );
                let mut throughs_option = None;
                for (filter_concrete_type, throughs) in intermediate_representation_content
                    .filter_concrete_type_and_throughs
                    .iter()
                {
                    if filter_concrete_type
                        .kind
                        .contains(&computed_filter.type_kind)
                    {
                        throughs_option = Some(throughs.clone());
                    }
                }
                let r#type = concrete_or_else(&node.r#type.kind, || {
                    match throughs_option.as_ref().unwrap() {
                        Throughs::Array(through_node) => {
                            TypeKind::Array(Box::new(through_node.r#type.clone()))
                        }
                        Throughs::Tuple {
                            nodes_indexes: _,
                            nodes,
                        } => TypeKind::Tuple(
                            nodes
                                .iter()
                                .map(|node| node.r#type.clone())
                                .collect::<Vec<_>>()
                                .into(),
                        ),
                    }
                });
                Ok(IntermediateValueAndMetadata {
                    intermediate_value: IntermediateValue::Filter(Filter {
                        intermediate_representation_content: intermediate_representation_content
                            .clone(),
                        computed_filter: self
                            .compute_node(&intermediate_representation_content.filter, constants)?,
                        throughs: throughs_option.unwrap(),
                        constants: constants.clone(),
                        lockable_internals: Arc::new(RwLock::new(FilterLockableInternals {
                            already_processed_values_count: 0,
                            already_computed_values: Vec::new(),
                        })),
                    }),
                    type_kind: r#type,
                }
                .into())
            }
            Content::Fold {
                fold,
                fold_constant_name_clustered_index,
                starting_with,
                accumulating_in_constant_name_clustered_index,
                fold_concrete_type_and_throughs,
            } => {
                let computed_fold =
                    self.unroll_intermediate_value(&self.compute_node(fold, constants)?)?;
                let computed_fold_array =
                    computed_fold.as_ref().as_ref().unwrap().as_tuple().unwrap();
                let mut result = self.compute_node(starting_with, constants)?;
                let mut throughs_option = None;
                for (fold_concrete_type, throughs) in fold_concrete_type_and_throughs.iter() {
                    if fold_concrete_type
                        .kind
                        .contains(&Value::type_kind(&computed_fold))
                    {
                        throughs_option = Some(throughs.clone());
                    }
                }
                match throughs_option.unwrap() {
                    Throughs::Array(through_node) => {
                        for element in computed_fold_array.inner.iter() {
                            let mut through_constants = constants.clone();
                            through_constants[*fold_constant_name_clustered_index] = Some(
                                IntermediateValueAndMetadata {
                                    intermediate_value: IntermediateValue::Value(element.clone()),
                                    type_kind: Value::type_kind(element),
                                }
                                .into(),
                            );
                            through_constants[*accumulating_in_constant_name_clustered_index] =
                                Some(result.clone());
                            result = self.compute_node(&through_node, &through_constants)?;
                        }
                    }
                    Throughs::Tuple {
                        nodes_indexes,
                        nodes,
                    } => {
                        for (element_index, element) in computed_fold_array.inner.iter().enumerate()
                        {
                            let mut through_constants = constants.clone();
                            through_constants[*fold_constant_name_clustered_index] = Some(
                                IntermediateValueAndMetadata {
                                    intermediate_value: IntermediateValue::Value(element.clone()),
                                    type_kind: node.r#type.kind.clone(),
                                }
                                .into(),
                            );
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
                            current_concrete_type_and_next: intermediate_representation_content
                                .current_concrete_type_kind_and_next
                                .clone(),
                            next_constants,
                            already_computed_values: [computed_starting_with].into(),
                            already_fully_computed: false,
                        })),
                    }),
                    type_kind: node.r#type.kind.clone(),
                }
                .into())
            }
            Content::Loop(intermediate_representation::Loop {
                starting_with,
                current_constant_name_clustered_index,
                current_concrete_type_kind_and_next,
                r#while,
            }) => {
                let mut result = self.compute_node(starting_with, constants)?;
                let mut next_constants = constants.clone();
                loop {
                    next_constants[*current_constant_name_clustered_index] = Some(result.clone());
                    if let Some(r#while) = r#while
                        && !self
                            .unroll_intermediate_value(
                                &self.compute_node(r#while, &next_constants)?,
                            )?
                            .as_ref()
                            .as_ref()
                            .unwrap()
                            .as_bool()
                            .unwrap()
                    {
                        break Ok(result);
                    }
                    let next_node = current_concrete_type_kind_and_next
                        .iter()
                        .find_map(|(current_concrete_type_kind, possible_next_node)| {
                            if current_concrete_type_kind.contains(&result.type_kind) {
                                Some(possible_next_node)
                            } else {
                                None
                            }
                        })
                        .unwrap();
                    result = self.compute_node(next_node, &next_constants)?;
                }
            }
            Content::Pipe {
                pipe,
                as_constant_name_clustered_index_option,
            } => {
                let mut previous_pipe_element_computed_option = None;
                for pipe_element in pipe {
                    if let Some(as_constant_name_clustered_index) =
                        as_constant_name_clustered_index_option
                        && let Some(previous_pipe_element_computed) =
                            previous_pipe_element_computed_option
                    {
                        let mut pipe_element_constants = constants.clone();
                        pipe_element_constants[*as_constant_name_clustered_index] =
                            Some(previous_pipe_element_computed);
                        previous_pipe_element_computed_option =
                            Some(self.compute_node(pipe_element, &pipe_element_constants)?);
                    } else {
                        previous_pipe_element_computed_option =
                            Some(self.compute_node(pipe_element, constants)?);
                    };
                }
                Ok(previous_pipe_element_computed_option.unwrap())
            }
            Content::Try {
                r#try,
                or,
                as_constant_name_clustered_index,
            } => {
                let error_string: ropey::Rope = match self.compute_node(r#try, constants) {
                    Ok(computed_try) => match self.unroll_intermediate_value(&computed_try) {
                        Ok(computed_try_unrolled) => {
                            let type_kind = Value::type_kind(&computed_try_unrolled);
                            return Ok(IntermediateValueAndMetadata {
                                intermediate_value: IntermediateValue::Value(computed_try_unrolled),
                                type_kind,
                            }
                            .into());
                        }
                        Err(error) => error.to_string().into(),
                    },
                    Err(error) => error.to_string().into(),
                };
                let mut or_constants = constants.clone();
                or_constants[*as_constant_name_clustered_index] = Some(
                    IntermediateValueAndMetadata {
                        intermediate_value: IntermediateValue::Value(
                            Some(Value::String(error_string.clone())).into(),
                        ),
                        type_kind: TypeKind::LiteralString(error_string),
                    }
                    .into(),
                );
                self.compute_node(or, &or_constants)
            }
            Content::Object(object) => {
                let mut result = containers::Object::default();
                for (key, value) in object {
                    result.insert(key.clone(), self.compute_or_lazy(value, constants)?);
                }
                Ok(IntermediateValueAndMetadata {
                    intermediate_value: IntermediateValue::Object(result),
                    type_kind: node.r#type.kind.clone(),
                }
                .into())
            }
            Content::Value(value) => Ok(IntermediateValueAndMetadata {
                intermediate_value: unsafe {
                    IntermediateValue::Value(std::mem::transmute::<
                        Arc<Option<intermediate_representation::Value>>,
                        Arc<Option<Value>>,
                    >(value.clone()))
                },
                type_kind: node.r#type.kind.clone(),
            }
            .into()),
        }
    }
}
