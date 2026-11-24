use crate::extract::Input;
use crate::model::{Fact, Gpa};

pub fn gpa(_inp: &Input) -> Fact<Gpa> {
    Fact {
        value: Gpa::Unknown,
        evidence: None,
    }
}
