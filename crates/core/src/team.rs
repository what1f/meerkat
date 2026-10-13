use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TeamRole {
    External,
    Member,
    Lead,
}
impl TeamRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::External => "external",
            Self::Member => "member",
            Self::Lead => "lead",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "external" => Some(Self::External),
            "member" => Some(Self::Member),
            "lead" => Some(Self::Lead),
            _ => None,
        }
    }
}
