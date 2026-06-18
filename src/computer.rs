use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::hash::Hash;
use std::sync::Arc;

use anyhow::Result;
use dashu::Rational;
use parking_lot::{Mutex, RwLock};
use rayon::prelude::*;

use crate::{
    containers::{Map, Vector},
    intermediate_representation::{Content, EmbeddedFunction, IntermediateRepresentation, Node},
    value::Value,
};

#[derive(Default)]
struct GlobalComputationContext {
    functions_results_cache: BTreeMap<u128, Value>,
}

#[derive(Clone, Debug)]
struct ComputationContext {
    constants: Vector<Value>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Computer {
    pub user_functions_caching: bool,
}

impl Computer {
    pub fn compute(
        &self,
        intermediate_representation: &IntermediateRepresentation,
    ) -> Result<Value> {
        self.compute_node(
            &intermediate_representation.root,
            intermediate_representation,
            &ComputationContext {
                constants: Vector {
                    inner: rpds::VectorSync::from_iter(
                        std::iter::repeat(Value::Null)
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
    ) -> Result<Value>
    where
        N: Iterator<Item = &'a Node>,
    {
        let mut result = vec![Value::Null; nodes_count];
        let complex_elements = nodes_iterator
            .enumerate()
            .filter(|(element_index, element)| match &element.content {
                Content::Value(value) => {
                    result[*element_index] = value.clone();
                    false
                }
                _ => true,
            })
            .collect::<Vec<_>>();
        Ok(Value::Array(Vector {
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
        }))
    }

    fn compute_node(
        &self,
        node: &Node,
        intermediate_representation: &IntermediateRepresentation,
        computation_context: &ComputationContext,
        global_computation_context: &Arc<RwLock<GlobalComputationContext>>,
    ) -> Result<Value> {
        match &node.content {
            Content::Array(array) => self.compute_nodes(
                array.iter(),
                array.len(),
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
                Ok(computation_context.constants.inner[*constant_name_clustered_index].clone())
            }
            Content::EmbeddedFunctionCall(embedded_function) => match &**embedded_function {
                EmbeddedFunction::Sum(argument) => Ok(Value::Number(
                    self.compute_node(
                        argument,
                        intermediate_representation,
                        computation_context,
                        global_computation_context,
                    )?
                    .as_array()
                    .unwrap()
                    .inner
                    .iter()
                    .fold(Rational::ZERO, |accumulator, current| {
                        accumulator + current.as_number().unwrap()
                    }),
                )),
                EmbeddedFunction::IsSorted(argument) => Ok(Value::Bool(
                    self.compute_node(
                        argument,
                        intermediate_representation,
                        computation_context,
                        global_computation_context,
                    )?
                    .as_array()
                    .unwrap()
                    .inner
                    .iter()
                    .is_sorted(),
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
                if self.user_functions_caching {
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
                Ok(Value::Object(result))
            }
            Content::Value(value) => Ok(value.clone()),
        }
    }
}
