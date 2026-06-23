use std::hash::Hash;
use std::sync::Arc;
use std::{collections::BTreeMap, io::Read};

use anyhow::{Context, Result};
use dashu::Rational;
use parking_lot::{Mutex, RwLock};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

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
        nodes_iterator: N,
        nodes_count: usize,
        intermediate_representation: &IntermediateRepresentation,
        computation_context: &ComputationContext,
        global_computation_context: &Arc<RwLock<GlobalComputationContext>>,
    ) -> Result<Option<Value>>
    where
        N: Iterator<Item = &'a Node>,
    {
        let mut result = vec![None; nodes_count];
        let complex_elements = nodes_iterator
            .enumerate()
            .filter(|(element_index, element)| match &element.content {
                Content::Value(value) => {
                    result[*element_index] = unsafe { std::mem::transmute(value.clone()) };
                    false
                }
                _ => true,
            })
            .collect::<Vec<_>>();
        Ok(Some(Value::Tuple(Vector {
            inner: rpds::VectorSync::from_iter(
                match complex_elements.len() {
                    0 => result,
                    1 => {
                        let (element_index, element_node) =
                            complex_elements.into_iter().next().unwrap();
                        result[element_index] = self.compute_node(
                            &element_node,
                            intermediate_representation,
                            computation_context,
                            global_computation_context,
                        )?;
                        result
                    }
                    2.. => {
                        let result_mutex = Mutex::new(result);
                        complex_elements
                            .into_par_iter()
                            .try_for_each(|(element_index, element_node)| {
                                self.compute_node(
                                    &element_node,
                                    intermediate_representation,
                                    computation_context,
                                    global_computation_context,
                                )
                                .and_then(|result| {
                                    result_mutex.lock()[element_index] = result;
                                    Ok(())
                                })
                            })
                            .and_then(|_| Ok(result_mutex.into_inner()))?
                    }
                }
                .into_iter(),
            ),
        })))
    }

    fn compute_node(
        &self,
        node: &Node,
        intermediate_representation: &IntermediateRepresentation,
        computation_context: &ComputationContext,
        global_computation_context: &Arc<RwLock<GlobalComputationContext>>,
    ) -> Result<Option<Value>> {
        match &node.content {
            Content::Tuple(tuple) => self.compute_nodes(
                tuple.iter(),
                tuple.len(),
                intermediate_representation,
                computation_context,
                global_computation_context,
            ),
            Content::Scope { constants, compute } => {
                let mut result_computation_context = computation_context.clone();
                for constant_definition in constants {
                    result_computation_context.constants.inner
                        [constant_definition.name_clustered_index] = self.compute_node(
                        &intermediate_representation.constants[constant_definition.index],
                        intermediate_representation,
                        &computation_context,
                        global_computation_context,
                    )?;
                }
                self.compute_node(
                    &compute,
                    intermediate_representation,
                    &result_computation_context,
                    global_computation_context,
                )
            }
            Content::Branching(branching) => {
                if self
                    .compute_node(
                        &branching.r#if,
                        intermediate_representation,
                        &computation_context,
                        global_computation_context,
                    )?
                    .unwrap()
                    .as_bool()
                    .unwrap()
                {
                    self.compute_node(
                        &branching.then,
                        intermediate_representation,
                        &computation_context,
                        global_computation_context,
                    )
                } else {
                    self.compute_node(
                        &branching.r#else,
                        intermediate_representation,
                        &computation_context,
                        global_computation_context,
                    )
                }
            }
            Content::Constant(constant_name_clustered_index) => {
                let result =
                    computation_context.constants.inner[*constant_name_clustered_index].clone();
                Ok(result)
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
                    .as_array()
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
                    .as_array()
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
            },
            Content::UserFunctionCall { arguments, body } => {
                let mut result_computation_context = computation_context.clone();
                for constant_definition in arguments {
                    let new_constant_value = self.compute_node(
                        &intermediate_representation.constants[constant_definition.index],
                        intermediate_representation,
                        &computation_context,
                        global_computation_context,
                    )?;
                    result_computation_context.constants.inner
                        [constant_definition.name_clustered_index] = new_constant_value;
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
            Content::Match { r#match, cases } => {
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
                                let mut case_computation_context = computation_context.clone();
                                case_computation_context.constants.inner
                                    [case.match_constant_definition.name_clustered_index] =
                                    computed_match;
                                return self.compute_node(
                                    &case.node,
                                    intermediate_representation,
                                    &case_computation_context,
                                    global_computation_context,
                                );
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
                                let mut case_computation_context = computation_context.clone();
                                case_computation_context.constants.inner
                                    [case.match_constant_definition.name_clustered_index] =
                                    computed_match;
                                return self.compute_node(
                                    &case.node,
                                    intermediate_representation,
                                    &case_computation_context,
                                    global_computation_context,
                                );
                            }
                        }
                    }
                }
                panic!()
            }
            Content::Object(object) => {
                let mut result = Map::default();
                for (key, value) in object {
                    result.inner.insert_mut(
                        key.clone(),
                        self.compute_node(
                            &value,
                            intermediate_representation,
                            &computation_context,
                            global_computation_context,
                        )?,
                    );
                }
                Ok(Some(Value::Object(result)))
            }
            Content::Value(value) => Ok(unsafe { std::mem::transmute(value.clone()) }),
        }
    }
}
