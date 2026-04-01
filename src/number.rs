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
               terms:
                 - Sum:
                     terms:
                       - 1
                       - 2
                 - 3",
            6f64,
        );
        execute_and_assert_number(
            "Sum:
               terms:
                 - Sum:
                     terms:
                       - 1.2
                       - 2.3
                 - 3.4",
            6.9f64,
        );
        execute_and_assert_number(
            "Sum:
               terms:
                 - Multiply:
                     terms:
                       - 1
                       - 2
                 - 3",
            5f64,
        );
    }
}
