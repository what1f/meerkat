use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MessageAuthorType {
    Human,
    Agent,
    Bot,
    System,
}

impl MessageAuthorType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Human => "human",
            Self::Agent => "agent",
            Self::Bot => "bot",
            Self::System => "system",
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Eq, PartialEq)]
pub enum ReactionAction {
    Add,
    Remove,
}
