use anyhow::Result;
use paste::paste;
use serde::Deserialize;

use crate::define_type_functions;

define_type_functions!(
    Number,
    f64,
    Sum {
        terms: Vec<ValueNumber>
    } self {
        let mut result = 0f64;
        for term in self.terms.iter() {
            result += term.compute()?;
        }
        Ok(result)
    }
    Multiply {
        terms: Vec<ValueNumber>
    } self {
        let mut result = 1f64;
        for term in self.terms.iter() {
            result *= term.compute()?;
        }
        Ok(result)
    }
);
