use std::sync::Mutex;

use anyhow::Result;
use dashu::Rational;
use rayon::prelude::*;

use crate::{
    containers::{Map, Vector},
    intermediate_representation::{Content, EmbeddedFunction, IntermediateRepresentation, Node},
    value::Value,
};

#[derive(Clone, Debug)]
struct ComputationContext {
    constants: Vector<Value>,
}

pub fn compute(intermediate_representation: &IntermediateRepresentation) -> Result<Value> {
    compute_node(
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
    )
}

fn compute_nodes<'a, N>(
    nodes_iterator: N,
    nodes_count: usize,
    intermediate_representation: &IntermediateRepresentation,
    computation_context: &ComputationContext,
) -> Result<Value>
where
    N: Iterator<Item = &'a Node>,
{
    let mut result = Vector {
        inner: rpds::VectorSync::from_iter(std::iter::repeat(Value::Null).take(nodes_count)),
    };
    let complex_elements = nodes_iterator
        .enumerate()
        .filter(|(element_index, element)| match &element.content {
            Content::Value(value) => {
                result.inner.set_mut(*element_index, value.clone());
                false
            }
            _ => true,
        })
        .collect::<Vec<_>>();
    Ok(match complex_elements.len() {
        0 => Value::Array(result),
        1 => {
            let (element_index, element_node) = complex_elements.into_iter().next().unwrap();
            result.inner.set_mut(
                element_index,
                compute_node(
                    &element_node,
                    intermediate_representation,
                    computation_context,
                )?,
            );
            Value::Array(result)
        }
        2.. => {
            let result_mutex = Mutex::new(result);
            complex_elements
                .into_par_iter()
                .try_for_each(|(element_index, element_node)| {
                    compute_node(
                        &element_node,
                        intermediate_representation,
                        computation_context,
                    )
                    .and_then(|result| {
                        result_mutex
                            .lock()
                            .unwrap()
                            .inner
                            .set_mut(element_index, result);
                        Ok(())
                    })
                })
                .and_then(|_| Ok(Value::Array(result_mutex.into_inner().unwrap())))?
        }
    })
}

fn compute_node(
    node: &Node,
    intermediate_representation: &IntermediateRepresentation,
    computation_context: &ComputationContext,
) -> Result<Value> {
    match &node.content {
        Content::Array(array) => compute_nodes(
            array.iter(),
            array.len(),
            intermediate_representation,
            computation_context,
        ),
        Content::Scope { constants, compute } => {
            let mut result_computation_context = computation_context.clone();
            for (constant_name_clustered_index, constant_index) in constants {
                result_computation_context.constants.inner[*constant_name_clustered_index] =
                    compute_node(
                        &intermediate_representation.constants[*constant_index],
                        intermediate_representation,
                        &computation_context,
                    )?;
            }
            compute_node(
                &compute,
                intermediate_representation,
                &result_computation_context,
            )
        }
        Content::Branching(branching) => {
            if compute_node(
                &branching.r#if,
                intermediate_representation,
                &computation_context,
            )?
            .as_bool()
            .unwrap()
            {
                compute_node(
                    &branching.then,
                    intermediate_representation,
                    &computation_context,
                )
            } else {
                compute_node(
                    &branching.r#else,
                    intermediate_representation,
                    &computation_context,
                )
            }
        }
        Content::Constant(constant_index) => {
            Ok(computation_context.constants.inner[*constant_index].clone())
        }
        Content::EmbeddedFunctionCall(embedded_function) => match &**embedded_function {
            EmbeddedFunction::Sum(argument) => Ok(Value::Number(
                compute_node(argument, intermediate_representation, computation_context)?
                    .as_array()
                    .unwrap()
                    .inner
                    .iter()
                    .map(|element| element.as_number().unwrap())
                    .fold(Rational::ZERO, |accumulator, current| accumulator + current),
            )),
            EmbeddedFunction::IsSorted(argument) => Ok(Value::Bool(
                compute_node(argument, intermediate_representation, computation_context)?
                    .as_array()
                    .unwrap()
                    .inner
                    .iter()
                    .is_sorted(),
            )),
        },
        Content::UserFunctionCall { arguments, body } => {
            let mut result_computation_context = computation_context.clone();
            for (constant_name_clustered_index, constant_index) in arguments {
                let new_constant_value = compute_node(
                    &intermediate_representation.constants[*constant_index],
                    intermediate_representation,
                    &computation_context,
                )?;
                result_computation_context.constants.inner[*constant_name_clustered_index] =
                    new_constant_value;
            }
            compute_node(
                &intermediate_representation.user_functions[*body],
                intermediate_representation,
                &result_computation_context,
            )
        }
        Content::Object(object) => {
            let mut result = Map {
                inner: rpds::RedBlackTreeMapSync::new_sync(),
            };
            for (key, value) in object {
                result.inner.insert_mut(
                    key.clone(),
                    compute_node(&value, intermediate_representation, &computation_context)?,
                );
            }
            Ok(Value::Object(result))
        }
        Content::Value(value) => Ok(value.clone()),
    }
}
