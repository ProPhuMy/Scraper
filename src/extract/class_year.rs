use crate::extract::Input;
use crate::model::{ClassYears, Fact};

pub fn class_years(_inp: &Input, _season_year: u16) -> Fact<ClassYears> {
    Fact {
        value: ClassYears::Unknown,
        evidence: None,
    }
}
