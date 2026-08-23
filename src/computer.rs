use std::{
    borrow::Cow,
    cell::LazyCell,
    collections::BTreeMap,
    hash::{Hash, Hasher},
    io::{BufWriter, Read, Write},
    ops::Range,
    str::FromStr,
    sync::{Arc, LazyLock},
};

use anyhow::{Context, Result, anyhow};
use dashu::Rational;
use gxhash::HashMap;
use parking_lot::{Mutex, RwLock};
use regex::{Regex, escape};
use serde::{Deserialize, Serialize};

use crate::{
    containers::{self, Object, Vector},
    intermediate_representation::{
        self, Condition, Content, IntermediateRepresentation, Node, RangeBound, Throughs,
        ValuePathSegment,
    },
    program::{EmbeddedFunction, Path},
    r#type::TypeKind,
    value::Value,
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
    built_regexes_cache: &'a Arc<RwLock<HashMap<u128, Arc<String>>>>,
    compiled_regexes_cache: &'a Arc<RwLock<HashMap<String, Arc<Regex>>>>,
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
            built_regexes_cache: &Arc::new(RwLock::new(HashMap::default())),
            compiled_regexes_cache: &Arc::new(RwLock::new(HashMap::default())),
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
    fn build_regex(&self, source: &Vector<Option<Value>>) -> Arc<String> {
        let source_hash = {
            let mut hasher = gxhash::GxHasher::default();
            source.hash(&mut hasher);
            hasher.finish_u128()
        };
        if let Some(result) = self.built_regexes_cache.read().get(&source_hash) {
            result.clone()
        } else {
            let mut result_string = String::new();
            for source_part in source.iter() {
                match source_part.as_ref().as_ref().unwrap() {
                    Value::String(string) if string == "character" => {
                        result_string.push('.');
                    }
                    Value::String(string) if string == "whitespace character" => {
                        result_string.push_str("\\s");
                    }
                    Value::String(string) if string == "non-whitespace character" => {
                        result_string.push_str("\\S");
                    }
                    Value::String(string) if string == "digit" => {
                        result_string.push_str("\\d");
                    }
                    Value::String(string) if string == "non-digit" => {
                        result_string.push_str("\\D");
                    }
                    Value::String(string) if string == "word character" => {
                        result_string.push_str("\\w");
                    }
                    Value::String(string) if string == "non-word character" => {
                        result_string.push_str("\\W");
                    }
                    Value::String(string) if string == "start of string" => {
                        result_string.push('^');
                    }
                    Value::String(string) if string == "end of string" => {
                        result_string.push('$');
                    }
                    Value::String(string) if string == "word boundary" => {
                        result_string.push_str("\\b");
                    }
                    Value::String(string) if string == "non-word boundary" => {
                        result_string.push_str("\\B");
                    }
                    Value::Object(object) => {
                        if let Some(variants) = object.get(&"one of".to_string()) {
                            result_string.push_str("(?:");
                            result_string.push_str(
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
                                    .map(|string_arc| (*string_arc).clone())
                                    .collect::<Vec<_>>()
                                    .join("|"),
                            );
                            result_string.push(')');
                        } else if let Some(string_of_characters_to_except) =
                            object.get(&"character except from string".to_string())
                        {
                            result_string.push_str("[^");
                            result_string.push_str(&escape(
                                &string_of_characters_to_except
                                    .as_ref()
                                    .as_ref()
                                    .unwrap()
                                    .as_string()
                                    .unwrap()
                                    .to_string(),
                            ));
                            result_string.push(']');
                        } else if let Some(raw_string) = object.get(&"raw string".to_string()) {
                            result_string.push_str(&escape(
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
                            result_string.push_str("(?P<");
                            result_string.push_str(
                                &name
                                    .as_ref()
                                    .as_ref()
                                    .unwrap()
                                    .as_string()
                                    .unwrap()
                                    .to_string(),
                            );
                            result_string.push('>');
                            result_string.push_str(
                                &self.build_regex(
                                    group.as_ref().as_ref().unwrap().as_tuple().unwrap(),
                                ),
                            );
                            result_string.push(')');
                        } else if let (Some(repeat), min, max, exactly) = (
                            object.get(&"repeat".to_string()),
                            object.get(&"min".to_string()),
                            object.get(&"max".to_string()),
                            object.get(&"exactly".to_string()),
                        ) {
                            result_string.push_str("(?:");
                            result_string.push_str(&self.build_regex(
                                repeat.as_ref().as_ref().unwrap().as_tuple().unwrap(),
                            ));
                            result_string.push_str("){");
                            if let Some(exactly) = exactly {
                                result_string.push_str(
                                    &(exactly
                                        .as_ref()
                                        .as_ref()
                                        .unwrap()
                                        .as_number()
                                        .unwrap()
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
                                        .to_f64()
                                        .value() as i64)
                                        .max(0)
                                } else {
                                    0
                                };
                                result_string.push_str(&min_number.to_string());
                                result_string.push(',');
                                if let Some(max) = max {
                                    result_string.push_str(
                                        &(max
                                            .as_ref()
                                            .as_ref()
                                            .unwrap()
                                            .as_number()
                                            .unwrap()
                                            .to_f64()
                                            .value()
                                            as i64)
                                            .max(min_number)
                                            .to_string(),
                                    );
                                }
                            }
                            result_string.push('}');
                        } else {
                            panic!()
                        }
                    }
                    _ => panic!(),
                }
            }
            let result = Arc::new(result_string);
            self.built_regexes_cache
                .write()
                .insert(source_hash, result.clone());
            result
        }
    }

    fn compile_regex(&self, source: &String) -> Result<Arc<Regex>> {
        if let Some(result) = self.compiled_regexes_cache.read().get(source) {
            Ok(result.clone())
        } else {
            let result = Arc::new(Regex::new(source)?);
            self.compiled_regexes_cache
                .write()
                .insert(source.clone(), result.clone());
            Ok(result)
        }
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
        let next = self.compute_node(next_node, &lockable_internals_write_guard.next_constants)?;
        lockable_internals_write_guard.next_constants[sequence
            .intermediate_representation_content
            .current_constant_name_clustered_index] = Some(next.clone());
        lockable_internals_write_guard
            .already_computed_values
            .push(next);
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

    fn get_by_key_from_intermediate_value(
        &self,
        intermediate_value_and_metadata: &Arc<IntermediateValueAndMetadata>,
        key: &String,
    ) -> Result<Option<Arc<IntermediateValueAndMetadata>>> {
        match &intermediate_value_and_metadata.intermediate_value {
            IntermediateValue::Value(value_arc) => match &**value_arc {
                Some(Value::Object(object)) => Ok(object.get(key).cloned().map(|value| {
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
            IntermediateValue::Object(object) => Ok(object.get(key).cloned()),
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
                    while lockable_internals_write_guard.already_computed_values.len() < range.end {
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
                return Ok(Vector::new_from_iter(
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
            &IntermediateValueAndMetadata {
                intermediate_value: IntermediateValue::Value(ref result),
                type_kind: _,
            } => Ok(result.clone()),
            &IntermediateValueAndMetadata {
                intermediate_value: IntermediateValue::Tuple(ref intermediate_values_list),
                type_kind: _,
            } => Ok(Some(Value::Tuple(self.unroll_intermediate_values(
                intermediate_values_list.inner.iter().cloned(),
                intermediate_values_list.len(),
            )?))
            .into()),
            &IntermediateValueAndMetadata {
                intermediate_value: IntermediateValue::Object(ref object),
                type_kind: _,
            } => Ok(Some(Value::Object(Object::new_from_iter(
                object.keys().cloned().zip(
                    self.unroll_intermediate_values(object.values().cloned(), object.len())?
                        .inner,
                ),
            )))
            .into()),
            &IntermediateValueAndMetadata {
                intermediate_value: IntermediateValue::Map(ref map),
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
            &IntermediateValueAndMetadata {
                intermediate_value: IntermediateValue::LazyValue(ref lazy_value),
                type_kind: _,
            } => self.unroll_intermediate_value(&self.compute_lazy_value(lazy_value)?),
            &IntermediateValueAndMetadata {
                intermediate_value: IntermediateValue::Filter(ref filter),
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
            unexpected_variant => Err(anyhow!("unexpected enum variant {unexpected_variant:#?}")),
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
                            Some(Value::Bytes(result.into())).into(),
                        ),
                        type_kind: TypeKind::Bytes,
                    }
                    .into())
                }
                EmbeddedFunction::ParseYaml => {
                    let result_value = serde_saphyr::from_str::<Option<Value>>(
                        &self
                            .unroll_intermediate_value(
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
                    let r#type = Value::type_kind(&result_value);
                    Ok(IntermediateValueAndMetadata {
                        intermediate_value: IntermediateValue::Value(result_value.into()),
                        type_kind: r#type,
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
                                        Some(Value::Tuple(Vector::new_from_iter(
                                            [
                                                Some(Value::String(ropey::Rope::from_str(key)))
                                                    .into(),
                                                value.clone(),
                                            ]
                                            .into_iter(),
                                        )))
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
                EmbeddedFunction::Flatten => {
                    Ok(IntermediateValueAndMetadata {
                        intermediate_value: IntermediateValue::Value(
                            Some(Value::Tuple({
                                let mut result = Vector::default();
                                for list in self
                                    .unroll_intermediate_value(&self.compute_node(
                                        &embedded_function_call.argument,
                                        constants,
                                    )?)?
                                    .as_ref()
                                    .as_ref()
                                    .unwrap()
                                    .as_tuple()
                                    .unwrap()
                                    .iter()
                                {
                                    result.append(
                                        list.as_ref()
                                            .as_ref()
                                            .unwrap()
                                            .as_tuple()
                                            .unwrap()
                                            .iter()
                                            .cloned(),
                                    );
                                }
                                result
                            }))
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
                    let computed_regex = computed_argument_unrolled
                        .as_ref()
                        .as_ref()
                        .unwrap()
                        .as_object()
                        .unwrap()
                        .get(&"regex".to_string())
                        .unwrap()
                        .as_ref()
                        .as_ref()
                        .unwrap();
                    let compiled_regex =
                        self.compile_regex(&*if let Value::String(computed_regex_string) =
                            computed_regex
                        {
                            Arc::new(computed_regex_string.to_string())
                        } else {
                            self.build_regex(computed_regex.as_tuple().unwrap())
                        })?;
                    if let Some(captures) = compiled_regex.captures(&computed_string) {
                        let result_value = Some(Value::Object(Object::new_from_iter(
                            [
                                (
                                    "groups".to_string().into(),
                                    Some(Value::Object(Object::new_from_iter(
                                        compiled_regex.capture_names().flatten().filter_map(
                                            |name| {
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
                                            },
                                        ),
                                    )))
                                    .into(),
                                ),
                                (
                                    "start".to_string().into(),
                                    Some(Value::Number(captures.get(0).unwrap().start().into()))
                                        .into(),
                                ),
                                (
                                    "end".to_string().into(),
                                    Some(Value::Number(captures.get(0).unwrap().end().into()))
                                        .into(),
                                ),
                            ]
                            .into_iter(),
                        )));
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
                EmbeddedFunction::ReadBytesFromFile => {
                    Ok(IntermediateValueAndMetadata {
                        intermediate_value: IntermediateValue::Value(
                            Some(Value::Bytes(
                                std::fs::read(
                                    self.unroll_intermediate_value(&self.compute_node(
                                        &embedded_function_call.argument,
                                        constants,
                                    )?)?
                                    .as_ref()
                                    .as_ref()
                                    .unwrap()
                                    .as_string()
                                    .unwrap()
                                    .to_string(),
                                )
                                .with_context(embedded_function_error_context(path_option))?
                                .into(),
                            ))
                            .into(),
                        ),
                        type_kind: TypeKind::Bytes,
                    }
                    .into())
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
                EmbeddedFunction::OverwriteFile => {
                    let computed_argument_unrolled = self.unroll_intermediate_value(
                        &self.compute_node(&embedded_function_call.argument, constants)?,
                    )?;
                    let computed_path = std::path::PathBuf::from(
                        computed_argument_unrolled
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
                            .to_string(),
                    );
                    let computed_content = computed_argument_unrolled
                        .as_ref()
                        .as_ref()
                        .unwrap()
                        .as_object()
                        .unwrap()
                        .get(&"content".to_string())
                        .unwrap()
                        .as_ref()
                        .as_ref()
                        .unwrap();
                    if let Some(parent) = computed_path.parent() {
                        std::fs::create_dir_all(parent)
                            .with_context(embedded_function_error_context(path_option))?;
                    }
                    match computed_content {
                        Value::String(string) => {
                            let file = std::fs::File::create(computed_path)
                                .with_context(embedded_function_error_context(path_option))?;
                            let mut writer = BufWriter::new(file);
                            for chunk in string.chunks() {
                                writer
                                    .write_all(chunk.as_bytes())
                                    .with_context(embedded_function_error_context(path_option))?;
                            }
                            writer
                                .flush()
                                .with_context(embedded_function_error_context(path_option))?;
                        }
                        Value::Bytes(bytes) => {
                            std::fs::write(computed_path, bytes)
                                .with_context(embedded_function_error_context(path_option))?;
                        }
                        _ => panic!(),
                    }
                    Ok(IntermediateValueAndMetadata {
                        intermediate_value: IntermediateValue::Value(None.into()),
                        type_kind: TypeKind::Null,
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
                EmbeddedFunction::CreateFile => {
                    let computed_argument_unrolled = self.unroll_intermediate_value(
                        &self.compute_node(&embedded_function_call.argument, constants)?,
                    )?;
                    let computed_path = std::path::PathBuf::from(
                        computed_argument_unrolled
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
                            .to_string(),
                    );
                    let computed_content = computed_argument_unrolled
                        .as_ref()
                        .as_ref()
                        .unwrap()
                        .as_object()
                        .unwrap()
                        .get(&"content".to_string())
                        .unwrap()
                        .as_ref()
                        .as_ref()
                        .unwrap();
                    if let Some(parent) = computed_path.parent() {
                        std::fs::create_dir_all(parent)
                            .with_context(embedded_function_error_context(path_option))?;
                    }
                    let file = std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&computed_path)
                        .with_context(embedded_function_error_context(path_option))?;
                    let mut writer = BufWriter::new(file);
                    match computed_content {
                        Value::String(string) => {
                            for chunk in string.chunks() {
                                writer
                                    .write_all(chunk.as_bytes())
                                    .with_context(embedded_function_error_context(path_option))?;
                            }
                        }
                        Value::Bytes(bytes) => {
                            writer
                                .write_all(bytes)
                                .with_context(embedded_function_error_context(path_option))?;
                        }
                        _ => panic!(),
                    }
                    writer
                        .flush()
                        .with_context(embedded_function_error_context(path_option))?;
                    Ok(IntermediateValueAndMetadata {
                        intermediate_value: IntermediateValue::Value(None.into()),
                        type_kind: TypeKind::Null,
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
                        })),
                    }),
                    type_kind: node.r#type.kind.clone(),
                }
                .into())
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
