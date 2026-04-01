use std::collections::BTreeMap;

use anyhow::Result;
use serde::Deserialize;

#[derive(Deserialize, Debug)]
pub struct Sum(pub Vec<ValueNumber>);

impl Sum {
    pub fn compute(&self) -> Result<f64> {
        let mut result = 0f64;
        for term in self.0.iter() {
            result += term.compute()?;
        }
        Ok(result)
    }
}

#[derive(Deserialize, Debug)]
pub struct Multiply(pub Vec<ValueNumber>);

impl Multiply {
    pub fn compute(&self) -> Result<f64> {
        let mut result = 1f64;
        for term in self.0.iter() {
            result *= term.compute()?;
        }
        Ok(result)
    }
}

#[derive(Deserialize, Debug)]
pub enum ComputableNumber {
    Sum(Sum),
    Multiply(Multiply),
}

#[derive(Deserialize, Debug)]
#[serde(untagged)]
pub enum ValueNumber {
    Computable(ComputableNumber),
    Raw(f64),
}

impl ValueNumber {
    pub fn compute(&self) -> Result<f64> {
        match self {
            ValueNumber::Computable(computable) => match computable {
                ComputableNumber::Sum(computable) => computable.compute(),
                ComputableNumber::Multiply(computable) => computable.compute(),
            },
            ValueNumber::Raw(raw_value) => Ok(raw_value.clone()),
        }
    }
}

#[derive(Deserialize, Debug)]
pub struct Concat(pub Vec<ValueString>);

impl Concat {
    pub fn compute(&self) -> Result<String> {
        let mut result = "".to_string();
        for string in self.0.iter() {
            result += &string.compute()?;
        }
        Ok(result)
    }
}

#[derive(Deserialize, Debug)]
pub struct Repeat {
    string: serde_saphyr::RcRecursive<ValueString>,
    amount: ValueNumber,
}

impl Repeat {
    pub fn compute(&self) -> Result<String> {
        let string = self.string.borrow().compute()?;
        let amount = self.amount.compute()? as usize;
        Ok(string.repeat(amount))
    }
}

#[derive(Deserialize, Debug)]
pub enum ComputableString {
    Concat(Concat),
    Repeat(Repeat),
}

#[derive(Deserialize, Debug)]
#[serde(untagged)]
pub enum ValueString {
    Computable(ComputableString),
    Raw(String),
}

impl ValueString {
    pub fn compute(&self) -> Result<String> {
        match self {
            ValueString::Computable(computable) => match computable {
                ComputableString::Concat(computable) => computable.compute(),
                ComputableString::Repeat(computable) => computable.compute(),
            },
            ValueString::Raw(raw_value) => Ok(raw_value.clone()),
        }
    }
}

pub enum Any {
    Number(ValueNumber),
    List(Vec<Any>),
    Dict(BTreeMap<ValueString, Any>),
}

#[cfg(test)]
mod tests {
    use super::*;

    use pretty_assertions::assert_eq;

    fn execute_and_assert_number<'a>(program_text: &'a str, correct_result: f64) {
        let program: ValueNumber = serde_saphyr::from_str::<'a>(program_text).unwrap();
        dbg!(&program);
        assert_eq!(program.compute().unwrap(), correct_result);
    }

    fn execute_and_assert_string<'a>(program_text: &'a str, correct_result: &str) {
        let program: ValueString = serde_saphyr::from_str::<'a>(program_text).unwrap();
        dbg!(&program);
        assert_eq!(&program.compute().unwrap(), correct_result);
    }

    #[test]
    fn test_example() {
        execute_and_assert_number(
            "Sum:
             - Sum:
               - 1
               - 2
             - 3",
            6f64,
        );
        execute_and_assert_number(
            "Sum:
             - Sum:
               - 1.2
               - 2.3
             - 3.4",
            6.9f64,
        );
        execute_and_assert_number(
            "Sum:
             - Multiply:
               - 1
               - 2
             - 3",
            5f64,
        );
        execute_and_assert_string(
            "Concat:
             - Repeat:
                 string: la
                 amount:
                   Sum:
                     - 0
                     - 2
             - lo",
            "lalalo",
        );
    }
}
