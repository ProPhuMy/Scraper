use crate::extract::Input;
use crate::model::{Experience, Fact};

pub fn experience(_inp: &Input) -> Fact<Experience> {
    Fact {
        value: Experience::Unknown,
        evidence: None,
    }
}
