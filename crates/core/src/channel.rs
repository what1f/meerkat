#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ChannelAbility {
    Reply,
    Attachment,
    Reaction,
    Edit,
    Recall,
}

impl ChannelAbility {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Reply => "reply",
            Self::Attachment => "attachment",
            Self::Reaction => "reaction",
            Self::Edit => "edit",
            Self::Recall => "recall",
        }
    }
}
