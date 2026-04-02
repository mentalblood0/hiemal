use anyhow::Result;
use paste::paste;
use serde::{Deserialize, Serialize};

use crate::define_type_functions;
use crate::number::ValueNumber;

define_type_functions!(
    String,
    String,
    Concat {
        strings: Vec<ValueString>
    } self {
        let mut result = "".to_string();
        for string in self.strings.iter() {
            result += &string.compute()?;
        }
        Ok(result)
    }
    Repeat {
        string: Box<ValueString>
        amount: ValueNumber
    } self {
        let string = self.string.compute()?;
        let amount = self.amount.compute()? as usize;
        Ok(string.repeat(amount))
    }
);
