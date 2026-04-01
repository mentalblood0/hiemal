use anyhow::Result;
use paste::paste;
use serde::Deserialize;

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
        string: serde_saphyr::RcRecursive<ValueString>
        amount: ValueNumber
    } self {
        let string = self.string.borrow().compute()?;
        let amount = self.amount.compute()? as usize;
        Ok(string.repeat(amount))
    }
);

#[cfg(test)]
mod tests {
    use super::*;

    use pretty_assertions::assert_eq;

    fn execute_and_assert_string<'a>(program_text: &'a str, correct_result: &str) {
        let program: ValueString = serde_saphyr::from_str::<'a>(program_text).unwrap();
        dbg!(&program);
        assert_eq!(&program.compute().unwrap(), correct_result);
    }

    #[test]
    fn test_example() {
        execute_and_assert_string(
            "Concat:
               strings:
               - Repeat:
                   string: la
                   amount:
                     Sum:
                       terms:
                         - 0
                         - 2
               - lo",
            "lalalo",
        );
    }
}
