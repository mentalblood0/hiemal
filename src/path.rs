#[derive(Debug)]
pub enum PathSegment {
    ObjectKey(String),
    Constant(String),
    Function(String),
    Argument(String),
    EmbeddedFunction(String),
    ArrayIndex(usize),
    With,
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
}

#[derive(Debug)]
pub struct Path(pub Vec<PathSegment>);
