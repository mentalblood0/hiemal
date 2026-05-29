use crate::clause::IncludeFromAt;

#[derive(Debug, serde::Deserialize)]
pub enum PathSegment {
    ObjectKey(String),
    Constant(String),
    Function(String),
    Argument(String),
    EmbeddedFunction(String),
    ArrayIndex(usize),
    With,
    Functions,
    Constants,
    Compute,
    Map,
    Filter,
    Fold,
    Through,
    StartingWith,
    If,
    Then,
    Else,
    Try,
    Or,
    From,
    At,
    AtIndex(usize),
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
