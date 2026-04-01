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

#[cfg(test)]
mod tests {
    use super::*;

    use pretty_assertions::assert_eq;

    fn execute_and_assert_number<'a>(program_text: &'a str, correct_result: f64) {
        let program: ValueNumber = serde_saphyr::from_str::<'a>(program_text).unwrap();
        dbg!(&program);
        assert_eq!(program.compute().unwrap(), correct_result);
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
    }
}
