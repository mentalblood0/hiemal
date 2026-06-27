use std::borrow::Cow;
use std::hash::Hash;
use std::sync::Arc;
use std::{collections::BTreeMap, io::Read};

use anyhow::{Context, Result};
use dashu::Rational;
use parking_lot::{Mutex, RwLock};
use rayon::prelude::*;
use rpds::RedBlackTreeMapSync;
use serde::{Deserialize, Serialize};

use crate::intermediate_representation::MapThroughs;
use crate::{
    containers::{Map, Vector},
    intermediate_representation::{
        Condition, Content, EmbeddedFunction, IntermediateRepresentation, Node, ValuePathSegment,
    },
    value::Value,
};

#[derive(Default)]
struct GlobalComputationContext {
    functions_results_cache: BTreeMap<u128, Option<Value>>,
}

#[derive(Clone, Debug)]
struct ComputationContext {
    constants: Vector<Option<Value>>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
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
                constants: Vector {
                    inner: rpds::VectorSync::from_iter(
                        std::iter::repeat(None)
                            .take(intermediate_representation.unique_constants_names_count),
                    ),
                },
            },
            &Arc::new(RwLock::new(GlobalComputationContext::default())),
        )
    }

    fn compute_nodes<'a, N>(
        &self,
        nodes_and_computation_contexts_iterator: N,
        nodes_count: usize,
        intermediate_representation: &IntermediateRepresentation,
        global_computation_context: &Arc<RwLock<GlobalComputationContext>>,
    ) -> Result<Vec<Option<Value>>>
    where
        N: Iterator<Item = (&'a Node, Cow<'a, ComputationContext>)>,
    {
        let mut result = vec![None; nodes_count];
        let complex_elements = nodes_and_computation_contexts_iterator
            .enumerate()
            .filter(
                |(element_index, (node, computation_context))| match &node.content {
                    Content::Value(value) => {
                        result[*element_index] = unsafe { std::mem::transmute(value.clone()) };
                        false
                    }
                    Content::Constant(constant_name_clustered_index) => {
                        result[*element_index] = computation_context.constants.inner
                            [*constant_name_clustered_index]
                            .clone();
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
                result[element_index] = self.compute_node(
                    node,
                    intermediate_representation,
                    &computation_context,
                    global_computation_context,
                )?;
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
                        .and_then(|result| {
                            result_mutex.lock()[element_index] = result;
                            Ok(())
                        })
                    })
                    .and_then(|_| Ok(result_mutex.into_inner()))?
            }
        })
    }

    fn compute_node(
        &self,
        node: &Node,
        intermediate_representation: &IntermediateRepresentation,
        computation_context: &ComputationContext,
        global_computation_context: &Arc<RwLock<GlobalComputationContext>>,
    ) -> Result<Option<Value>> {
        match &node.content {
            Content::Tuple(tuple) => Ok(Some(Value::Tuple(Vector {
                inner: rpds::VectorSync::from_iter(
                    self.compute_nodes(
                        tuple
                            .iter()
                            .map(|node| (node, Cow::Borrowed(computation_context))),
                        tuple.len(),
                        intermediate_representation,
                        global_computation_context,
                    )?,
                ),
            }))),
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
                    result_computation_context.constants.inner[constant_name_clustered_index] =
                        computed_constant;
                }
                self.compute_node(
                    &compute,
                    intermediate_representation,
                    &result_computation_context,
                    global_computation_context,
                )
            }
            Content::Constant(constant_name_clustered_index) => {
                Ok(computation_context.constants.inner[*constant_name_clustered_index].clone())
            }
            Content::EmbeddedFunctionCall {
                path,
                embedded_function,
            } => match &**embedded_function {
                EmbeddedFunction::Sum(argument) => Ok(Some(Value::Number(
                    self.compute_node(
                        argument,
                        intermediate_representation,
                        computation_context,
                        global_computation_context,
                    )?
                    .unwrap()
                    .as_tuple()
                    .unwrap()
                    .inner
                    .iter()
                    .fold(Rational::ZERO, |accumulator, current| {
                        accumulator + current.as_ref().unwrap().as_number().unwrap()
                    }),
                ))),
                EmbeddedFunction::IsSorted(argument) => Ok(Some(Value::Bool(
                    self.compute_node(
                        argument,
                        intermediate_representation,
                        computation_context,
                        global_computation_context,
                    )?
                    .unwrap()
                    .as_tuple()
                    .unwrap()
                    .inner
                    .iter()
                    .is_sorted(),
                ))),
                EmbeddedFunction::StandardInput => {
                    let mut result = String::new();
                    std::io::stdin()
                        .read_to_string(&mut result)
                        .with_context(|| {
                            format!("can not compute embedded function at path {:#?}", path)
                        })?;
                    Ok(Some(Value::String(ropey::Rope::from(result))))
                }
                EmbeddedFunction::ParseYaml(argument) => Ok(Some(
                    serde_saphyr::from_str::<Value>(
                        &self
                            .compute_node(
                                argument,
                                intermediate_representation,
                                computation_context,
                                global_computation_context,
                            )?
                            .unwrap()
                            .as_string()
                            .unwrap()
                            .to_string(),
                    )
                    .with_context(|| {
                        format!("can not compute embedded function at path {:#?}", path)
                    })?,
                )),
                EmbeddedFunction::KeyValuePairs(argument) => Ok(Some(Value::Tuple(Vector {
                    inner: rpds::VectorSync::from_iter(
                        self.compute_node(
                            argument,
                            intermediate_representation,
                            computation_context,
                            global_computation_context,
                        )?
                        .unwrap()
                        .as_object()
                        .unwrap()
                        .inner
                        .iter()
                        .map(|(key, value)| {
                            Some(Value::Tuple(Vector {
                                inner: rpds::VectorSync::from_iter([
                                    Some(Value::String(ropey::Rope::from_str(key))),
                                    value.clone(),
                                ]),
                            }))
                        }),
                    ),
                }))),
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
                    result_computation_context.constants.inner[constant_name_clustered_index] =
                        computed_constant;
                }
                let user_function = &intermediate_representation.user_functions[*body];
                if self.user_functions_caching && user_function.is_pure {
                    let function_call_identifier = {
                        let mut hasher = gxhash::GxHasher::default();
                        for constant_name_clustered_index in
                            &user_function.external_constants_name_clustered_indices
                        {
                            let constant_value = &result_computation_context.constants.inner
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
                let mut result = self.compute_node(
                    from,
                    intermediate_representation,
                    computation_context,
                    global_computation_context,
                )?;
                for path_segment in value_path_segments {
                    match path_segment {
                        ValuePathSegment::ArrayIndex(array_index) => {
                            result = std::mem::take(
                                result
                                    .unwrap()
                                    .as_array_mut()
                                    .unwrap()
                                    .inner
                                    .get_mut(*array_index)
                                    .unwrap(),
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
                    }
                }
                Ok(result)
            }
            Content::Match {
                r#match,
                cases,
                match_constant_name_clustered_index_option,
            } => {
                let computed_match = self.compute_node(
                    r#match,
                    intermediate_representation,
                    computation_context,
                    global_computation_context,
                )?;
                let match_type = Value::r#type(&computed_match);
                for case in cases {
                    match &case.condition {
                        Condition::Type(expected_type) => {
                            if expected_type.contains(&match_type) {
                                if let Some(match_constant_name_clustered_index) =
                                    match_constant_name_clustered_index_option
                                {
                                    let mut case_computation_context = computation_context.clone();
                                    case_computation_context.constants.inner
                                        [*match_constant_name_clustered_index] = computed_match;
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
                            let computed_expected_value = self.compute_node(
                                &expected_value_node,
                                intermediate_representation,
                                computation_context,
                                global_computation_context,
                            )?;
                            if computed_expected_value == computed_match {
                                if let Some(match_constant_name_clustered_index) =
                                    match_constant_name_clustered_index_option
                                {
                                    let mut case_computation_context = computation_context.clone();
                                    case_computation_context.constants.inner
                                        [*match_constant_name_clustered_index] = computed_match;
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
                let computed_map = self.compute_node(
                    &map,
                    intermediate_representation,
                    computation_context,
                    global_computation_context,
                )?;
                let computed_map_array = computed_map.as_ref().unwrap().as_tuple().unwrap();
                match throughs {
                    MapThroughs::Array(node) => Ok(Some(Value::Tuple(Vector {
                        inner: rpds::VectorSync::from_iter(self.compute_nodes(
                            computed_map_array.inner.iter().map(|element_value| {
                                let mut element_computation_context = computation_context.clone();
                                element_computation_context.constants.inner
                                    [*map_constant_name_clustered_index] = element_value.clone();
                                (&**node, Cow::Owned(element_computation_context))
                            }),
                            computed_map_array.inner.len(),
                            intermediate_representation,
                            global_computation_context,
                        )?),
                    }))),
                    MapThroughs::Tuple {
                        elements_nodes_indexes,
                        nodes,
                    } => Ok(Some(Value::Tuple(Vector {
                        inner: rpds::VectorSync::from_iter(self.compute_nodes(
                            computed_map_array.inner.iter().enumerate().map(
                                |(element_index, element_value)| {
                                    let mut element_computation_context =
                                        computation_context.clone();
                                    element_computation_context.constants.inner
                                        [*map_constant_name_clustered_index] =
                                        element_value.clone();
                                    (
                                        &nodes[elements_nodes_indexes[element_index]],
                                        Cow::Owned(element_computation_context),
                                    )
                                },
                            ),
                            computed_map_array.inner.len(),
                            intermediate_representation,
                            global_computation_context,
                        )?),
                    }))),
                }
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
            Content::Value(value) => Ok(unsafe { std::mem::transmute(value.clone()) }),
        }
    }
}
