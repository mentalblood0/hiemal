use crate::clause::IncludeFromAt;

#[derive(Debug, serde::Deserialize)]
pub enum PathSegment {
    #[serde(rename = "object key")]
    ObjectKey(String),
    #[serde(rename = "constant")]
    Constant(String),
    #[serde(rename = "function")]
    Function(String),
    #[serde(rename = "argument")]
    Argument(String),
    #[serde(rename = "embedded function")]
    EmbeddedFunction(String),
    #[serde(rename = "array index")]
    ArrayIndex(usize),
    #[serde(rename = "with")]
    With,
    #[serde(rename = "functions")]
    Functions,
    #[serde(rename = "constants")]
    Constants,
    #[serde(rename = "compute")]
    Compute,
    #[serde(rename = "map")]
    Map,
    #[serde(rename = "as")]
    As,
    #[serde(rename = "filter")]
    Filter,
    #[serde(rename = "fold")]
    Fold,
    #[serde(rename = "accumulating in")]
    AccumulatingIn,
    #[serde(rename = "through")]
    Through,
    #[serde(rename = "starting with")]
    StartingWith,
    #[serde(rename = "if")]
    If,
    #[serde(rename = "then")]
    Then,
    #[serde(rename = "else")]
    Else,
    #[serde(rename = "try")]
    Try,
    #[serde(rename = "or")]
    Or,
    #[serde(rename = "with error")]
    WithError,
    #[serde(rename = "from")]
    From,
    #[serde(rename = "at")]
    At,
    #[serde(rename = "at index")]
    AtIndex(usize),
    #[serde(rename = "include")]
    Include(IncludeFromAt),
}

#[derive(Clone, serde::Deserialize)]
pub struct Path(pub rpds::VectorSync<PathSegment>);

impl std::fmt::Debug for Path {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let collected: Vec<&PathSegment> = self.0.iter().collect();
        f.debug_tuple("Path").field(&collected).finish()
    }
}
