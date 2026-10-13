#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DesktopLanguage {
    #[default]
    SimplifiedChinese,
    English,
}

impl DesktopLanguage {
    pub fn text(self, simplified_chinese: &'static str, english: &'static str) -> &'static str {
        match self {
            Self::SimplifiedChinese => simplified_chinese,
            Self::English => english,
        }
    }
}
