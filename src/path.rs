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

#[derive(Clone)]
pub struct Path(pub rpds::Vector<PathSegment>);

impl std::fmt::Debug for Path {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let collected: Vec<&PathSegment> = self.0.iter().collect();
        f.debug_tuple("Path").field(&collected).finish()
    }
}
