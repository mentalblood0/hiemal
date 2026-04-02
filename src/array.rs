use anyhow::Result;
use paste::paste;
use serde::{Deserialize, Serialize};

use crate::define_type_functions;
use crate::string::ValueString;

define_type_functions!(
    StringArray,
    Vec<String>,
    Split {
        string: ValueString
        delimiter: ValueString
    } self {
        let string = self.string.compute()?;
        let delimiter = self.delimiter.compute()?;
        Ok(string.split(&delimiter).map(|s| s.to_string()).collect())
    }
);
