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
    computer: Arc<Computer>,
    node: Arc<Node>,
    intermediate_representation: Arc<IntermediateRepresentation>,
    computation_context: ComputationContext,
    global_computation_context: Arc<RwLock<GlobalComputationContext>>,
}

impl LazyValue {
    fn compute(&self) -> Result<IntermediateValue> {
        self.computer.compute_node(
            &self.node,
            &self.intermediate_representation,
            &self.computation_context,
            &self.global_computation_context,
        )
    }
}

#[derive(Clone, Debug)]
struct SequenceLockableInternals {
    next_lazy_value: Option<LazyValue>,
    already_computed_values: List<IntermediateValue>,
}

#[derive(Clone, Debug)]
struct Sequence {
    intermediate_representation_content: intermediate_representation::Sequence,
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

impl Sequence {
    fn next(&self) -> Result<IntermediateValue> {
        let mut lockable_internals_write_guard = self.lockable_internals.write();
        let mut current_next_lazy_value =
            std::mem::take(&mut lockable_internals_write_guard.next_lazy_value).unwrap();
        let result = current_next_lazy_value.compute()?;
        current_next_lazy_value.computation_context.constants[self
            .intermediate_representation_content
            .current_constant_name_clustered_index] = Some(result.clone());
        lockable_internals_write_guard.next_lazy_value = Some(current_next_lazy_value);
        lockable_internals_write_guard
            .already_computed_values
            .push_back_mut(result.clone());
        Ok(result)
    }
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
enum IntermediateValue {
    Value(Option<Value>),
    Tuple(List<IntermediateValue>),
    Sequence(Sequence),
}

impl IntermediateValue {
    fn value(self) -> Result<Option<Value>> {
        match self {
            IntermediateValue::Value(result) => Ok(result),
            IntermediateValue::Tuple(intermediate_values_list) => {
                let mut result = List::default();
                for intermediate_value in intermediate_values_list.inner.into_iter() {
                    result.push_back_mut(intermediate_value.value()?);
                }
                Ok(Some(Value::Tuple(result)))
            }
            IntermediateValue::Sequence(_) => Err(anyhow!(
                "expected value, tuple or object, got unlimited sequence"
            )),
        }
    }
}

#[derive(Default, Debug)]
struct GlobalComputationContext {
    functions_results_cache: HashMap<u128, IntermediateValue>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct ComputationContext {
    constants: rpds::VectorSync<Option<IntermediateValue>>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct Computer {
    pub user_functions_caching: bool,
}

impl Computer {
    pub fn compute(
        &self,
        intermediate_representation: &IntermediateRepresentation,
    ) -> Result<Option<Value>> {
        self.compute_node(
            &intermediate_representation.root,
            intermediate_representation,
            &ComputationContext {
                constants: rpds::VectorSync::from_iter(std::iter::repeat_n(
                    None,
                    intermediate_representation.unique_constants_names_count,
                )),
            },
            &Arc::new(RwLock::new(GlobalComputationContext::default())),
        )?
        .value()
    }

    fn compute_nodes<'a, N>(
        &self,
        nodes_and_computation_contexts_iterator: N,
        nodes_count: usize,
        intermediate_representation: &IntermediateRepresentation,
        global_computation_context: &Arc<RwLock<GlobalComputationContext>>,
    ) -> Result<Vec<IntermediateValue>>
    where
        N: Iterator<Item = (&'a Node, Cow<'a, ComputationContext>)>,
    {
        let mut result = vec![None; nodes_count];
        let complex_elements = nodes_and_computation_contexts_iterator
            .enumerate()
            .filter(
                |(element_index, (node, computation_context))| match &node.content {
                    Content::Value(value) => {
                        result[*element_index] = Some(IntermediateValue::Value(unsafe {
                            std::mem::transmute::<
                                Option<intermediate_representation::Value>,
                                Option<Value>,
                            >(value.clone())
                        }));
                        false
                    }
                    Content::Constant(constant_name_clustered_index) => {
                        result[*element_index] = Some(
                            computation_context.constants[*constant_name_clustered_index]
                                .clone()
                                .unwrap(),
                        );
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
                result[element_index] = Some(self.compute_node(
                    node,
                    intermediate_representation,
                    &computation_context,
                    global_computation_context,
                )?);
                result
            }
            2.. => {
                let result_mutex = Mutex::new(result);
                complex_elements
                    .into_par_iter()
                    .try_for_each(|(element_index, (node, computation_context))| {
                        self.compute_node(
                            node,
                            intermediate_representation,
                            &computation_context,
                            global_computation_context,
                        )
                        .map(|result| {
                            result_mutex.lock()[element_index] = Some(result);
                        })
                    })
                    .map(|_| result_mutex.into_inner())?
            }
        }
        .into_iter()
        .map(Option::unwrap)
        .collect())
    }

    fn compute_node(
        &self,
        node: &Node,
        intermediate_representation: &IntermediateRepresentation,
        computation_context: &ComputationContext,
        global_computation_context: &Arc<RwLock<GlobalComputationContext>>,
    ) -> Result<IntermediateValue> {
        match &node.content {
            Content::Tuple(tuple) => Ok(IntermediateValue::Tuple(List {
                inner: im_lists::list::SharedList::from_iter(
                    self.compute_nodes(
                        tuple
                            .iter()
                            .map(|node| (node, Cow::Borrowed(computation_context))),
                        tuple.len(),
                        intermediate_representation,
                        global_computation_context,
                    )?,
                ),
            })),
            Content::Scope { constants, compute } => {
                let mut result_computation_context = computation_context.clone();
                for (constant_name_clustered_index, computed_constant) in constants
                    .iter()
                    .map(|constant_definition| constant_definition.name_clustered_index)
                    .zip(self.compute_nodes(
                        constants.iter().map(|constant_definition| {
                            (
                                &constant_definition.node,
                                Cow::Borrowed(computation_context),
                            )
                        }),
                        constants.len(),
                        intermediate_representation,
                        global_computation_context,
                    )?)
                {
                    result_computation_context.constants[constant_name_clustered_index] =
                        Some(computed_constant);
                }
                self.compute_node(
                    compute,
                    intermediate_representation,
                    &result_computation_context,
                    global_computation_context,
                )
            }
            Content::Constant(constant_name_clustered_index) => Ok(computation_context.constants
                [*constant_name_clustered_index]
                .clone()
                .unwrap()),
            Content::EmbeddedFunctionCall {
                path,
                embedded_function,
            } => match &**embedded_function {
                EmbeddedFunction::Sum(argument) => {
                    Ok(IntermediateValue::Value(Some(Value::Number(
                        self.compute_node(
                            argument,
                            intermediate_representation,
                            computation_context,
                            global_computation_context,
                        )?
                        .value()?
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
                        self.compute_node(
                            argument,
                            intermediate_representation,
                            computation_context,
                            global_computation_context,
                        )?
                        .value()?
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
                            .compute_node(
                                argument,
                                intermediate_representation,
                                computation_context,
                                global_computation_context,
                            )?
                            .value()?
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
                            self.compute_node(
                                argument,
                                intermediate_representation,
                                computation_context,
                                global_computation_context,
                            )?
                            .value()?
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
                                self.compute_node(
                                    argument,
                                    intermediate_representation,
                                    computation_context,
                                    global_computation_context,
                                )?
                                .value()?
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
                let mut result_computation_context = computation_context.clone();
                for (constant_name_clustered_index, computed_constant) in arguments
                    .iter()
                    .map(|constant_definition| constant_definition.name_clustered_index)
                    .zip(self.compute_nodes(
                        arguments.iter().map(|constant_definition| {
                            (
                                &constant_definition.node,
                                Cow::Borrowed(computation_context),
                            )
                        }),
                        arguments.len(),
                        intermediate_representation,
                        global_computation_context,
                    )?)
                {
                    result_computation_context.constants[constant_name_clustered_index] =
                        Some(computed_constant);
                }
                let user_function = &intermediate_representation.user_functions[*body];
                if self.user_functions_caching && user_function.is_pure {
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
                    let global_computation_context_read_guard = global_computation_context.read();
                    if let Some(cached_function_result) = global_computation_context_read_guard
                        .functions_results_cache
                        .get(&function_call_identifier)
                    {
                        Ok(cached_function_result.clone())
                    } else {
                        drop(global_computation_context_read_guard);
                        let result = self.compute_node(
                            &user_function.node,
                            intermediate_representation,
                            &result_computation_context,
                            global_computation_context,
                        )?;
                        global_computation_context
                            .write()
                            .functions_results_cache
                            .insert(function_call_identifier, result.clone());
                        Ok(result)
                    }
                } else {
                    self.compute_node(
                        &user_function.node,
                        intermediate_representation,
                        &result_computation_context,
                        global_computation_context,
                    )
                }
            }
            Content::FromAt {
                from,
                value_path_segments,
            } => {
                let mut result = self
                    .compute_node(
                        from,
                        intermediate_representation,
                        computation_context,
                        global_computation_context,
                    )?
                    .value()?;
                for path_segment in value_path_segments {
                    match path_segment {
                        ValuePathSegment::ArrayIndex(array_index) => {
                            result = std::mem::take(
                                &mut result
                                    .unwrap()
                                    .as_tuple_mut()
                                    .unwrap()
                                    .inner
                                    .get(*array_index)
                                    .unwrap()
                                    .to_owned(),
                            )
                        }
                        ValuePathSegment::ObjectKey(object_key) => {
                            result = std::mem::take(
                                result
                                    .unwrap()
                                    .as_object_mut()
                                    .unwrap()
                                    .inner
                                    .get_mut(object_key)
                                    .unwrap(),
                            )
                        }
                        ValuePathSegment::ArrayRange((from, to)) => {
                            let from_number = match from {
                                RangeBound::Static(Some(from)) => *from as f64,
                                RangeBound::Static(None) => 0f64,
                                RangeBound::Dynamic(from_node) => self
                                    .compute_node(
                                        from_node,
                                        intermediate_representation,
                                        computation_context,
                                        global_computation_context,
                                    )?
                                    .value()?
                                    .unwrap()
                                    .as_number()
                                    .unwrap()
                                    .to_f64()
                                    .value(),
                            };
                            let to_number = match to {
                                RangeBound::Static(Some(to)) => *to as f64,
                                RangeBound::Static(None) => 0f64,
                                RangeBound::Dynamic(to_node) => self
                                    .compute_node(
                                        to_node,
                                        intermediate_representation,
                                        computation_context,
                                        global_computation_context,
                                    )?
                                    .value()?
                                    .unwrap()
                                    .as_number()
                                    .unwrap()
                                    .to_f64()
                                    .value(),
                            };
                            result = Some(Value::Tuple(List {
                                inner: im_lists::list::SharedList::from_iter(
                                    std::mem::take(
                                        &mut result.unwrap().as_tuple_mut().unwrap().inner,
                                    )
                                    .into_iter()
                                    .skip(from_number.max(0f64) as usize)
                                    .take((to_number - from_number).max(0f64) as usize),
                                ),
                            }))
                        }
                    }
                }
                Ok(IntermediateValue::Value(result))
            }
            Content::Match {
                r#match,
                cases,
                match_constant_name_clustered_index_option,
            } => {
                let computed_match = self
                    .compute_node(
                        r#match,
                        intermediate_representation,
                        computation_context,
                        global_computation_context,
                    )?
                    .value()?;
                let match_type = Value::r#type(&computed_match);
                for case in cases {
                    match &case.condition {
                        Condition::Type(expected_type) => {
                            if expected_type.contains(&match_type) {
                                if let Some(match_constant_name_clustered_index) =
                                    match_constant_name_clustered_index_option
                                {
                                    let mut case_computation_context = computation_context.clone();
                                    case_computation_context.constants
                                        [*match_constant_name_clustered_index] =
                                        Some(IntermediateValue::Value(computed_match));
                                    return self.compute_node(
                                        &case.node,
                                        intermediate_representation,
                                        &case_computation_context,
                                        global_computation_context,
                                    );
                                } else {
                                    return self.compute_node(
                                        &case.node,
                                        intermediate_representation,
                                        computation_context,
                                        global_computation_context,
                                    );
                                }
                            }
                        }
                        Condition::Value(expected_value_node) => {
                            let computed_expected_value = self
                                .compute_node(
                                    expected_value_node,
                                    intermediate_representation,
                                    computation_context,
                                    global_computation_context,
                                )?
                                .value()?;
                            if computed_expected_value == computed_match {
                                if let Some(match_constant_name_clustered_index) =
                                    match_constant_name_clustered_index_option
                                {
                                    let mut case_computation_context = computation_context.clone();
                                    case_computation_context.constants
                                        [*match_constant_name_clustered_index] =
                                        Some(IntermediateValue::Value(computed_match));
                                    return self.compute_node(
                                        &case.node,
                                        intermediate_representation,
                                        &case_computation_context,
                                        global_computation_context,
                                    );
                                } else {
                                    return self.compute_node(
                                        &case.node,
                                        intermediate_representation,
                                        computation_context,
                                        global_computation_context,
                                    );
                                }
                            }
                        }
                    }
                }
                panic!()
            }
            Content::Map {
                map,
                throughs,
                map_constant_name_clustered_index,
            } => {
                let computed_map = self
                    .compute_node(
                        map,
                        intermediate_representation,
                        computation_context,
                        global_computation_context,
                    )?
                    .value()?;
                let computed_map_array = computed_map.as_ref().unwrap().as_tuple().unwrap();
                match throughs {
                    Throughs::Array(node) => Ok(IntermediateValue::Tuple(List {
                        inner: im_lists::list::SharedList::from_iter(self.compute_nodes(
                            computed_map_array.inner.iter().map(|element_value| {
                                let mut through_computation_context = computation_context.clone();
                                through_computation_context.constants
                                    [*map_constant_name_clustered_index] =
                                    Some(IntermediateValue::Value(element_value.clone()));
                                (&**node, Cow::Owned(through_computation_context))
                            }),
                            computed_map_array.inner.len(),
                            intermediate_representation,
                            global_computation_context,
                        )?),
                    })),
                    Throughs::Tuple {
                        nodes_indexes,
                        nodes,
                    } => Ok(IntermediateValue::Tuple(List {
                        inner: im_lists::list::SharedList::from_iter(self.compute_nodes(
                            computed_map_array.inner.iter().enumerate().map(
                                |(element_index, element_value)| {
                                    let mut through_computation_context =
                                        computation_context.clone();
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
                            intermediate_representation,
                            global_computation_context,
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
                let computed_fold = self
                    .compute_node(
                        fold,
                        intermediate_representation,
                        computation_context,
                        global_computation_context,
                    )?
                    .value()?;
                let computed_fold_array = computed_fold.as_ref().unwrap().as_tuple().unwrap();
                let mut result = self.compute_node(
                    starting_with,
                    intermediate_representation,
                    computation_context,
                    global_computation_context,
                )?;
                match throughs {
                    Throughs::Array(through_node) => {
                        for element in computed_fold_array.inner.iter() {
                            let mut through_computation_context = computation_context.clone();
                            through_computation_context.constants
                                [*fold_constant_name_clustered_index] =
                                Some(IntermediateValue::Value(element.clone()));
                            through_computation_context.constants
                                [*accumulating_in_constant_name_clustered_index] =
                                Some(result.clone());
                            result = self.compute_node(
                                through_node,
                                intermediate_representation,
                                &through_computation_context,
                                global_computation_context,
                            )?;
                        }
                    }
                    Throughs::Tuple {
                        nodes_indexes,
                        nodes,
                    } => {
                        for (element_index, element) in computed_fold_array.inner.iter().enumerate()
                        {
                            let mut through_computation_context = computation_context.clone();
                            through_computation_context.constants
                                [*fold_constant_name_clustered_index] =
                                Some(IntermediateValue::Value(element.clone()));
                            through_computation_context.constants
                                [*accumulating_in_constant_name_clustered_index] =
                                Some(result.clone());
                            result = self.compute_node(
                                &nodes[nodes_indexes[element_index]],
                                intermediate_representation,
                                &through_computation_context,
                                global_computation_context,
                            )?;
                        }
                    }
                }
                Ok(result)
            }
            Content::Sequence(intermediate_representation::Sequence {
                starting_with,
                current_constant_name_clustered_index,
                next,
                r#while,
            }) => {
                let mut result = List::default();
                let mut current_computation_context = computation_context.clone();
                current_computation_context.constants[*current_constant_name_clustered_index] =
                    self.compute_node(
                        starting_with,
                        intermediate_representation,
                        computation_context,
                        global_computation_context,
                    )?;
                while self
                    .compute_node(
                        r#while,
                        intermediate_representation,
                        &current_computation_context,
                        global_computation_context,
                    )?
                    .unwrap()
                    .as_bool()
                    .unwrap()
                {
                    result.push_back_mut(
                        current_computation_context.constants
                            [*current_constant_name_clustered_index]
                            .clone(),
                    );
                    current_computation_context.constants[*current_constant_name_clustered_index] =
                        self.compute_node(
                            next,
                            intermediate_representation,
                            &current_computation_context,
                            global_computation_context,
                        )?;
                }
                Ok(Some(Value::Tuple(result)))
            }
            Content::Object(object) => Ok(Some(Value::Object(Map {
                inner: RedBlackTreeMapSync::from_iter(
                    object.keys().cloned().zip(
                        self.compute_nodes(
                            object
                                .values()
                                .map(|value| (value, Cow::Borrowed(computation_context))),
                            object.len(),
                            intermediate_representation,
                            global_computation_context,
                        )?,
                    ),
                ),
            }))),
            Content::Value(value) => Ok(unsafe {
                std::mem::transmute::<Option<intermediate_representation::Value>, Option<Value>>(
                    value.clone(),
                )
            }),
        }
    }
}
