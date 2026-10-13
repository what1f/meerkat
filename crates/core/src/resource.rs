#[derive(Clone, Debug, Default, Hash, Eq, PartialEq)]
pub struct ResourceRef {
    pub resource_id: i64,
    pub file_name: String,
    pub media_type: String,
    pub size_bytes: u64,
    pub sha256: String,
}

pub const PROVIDER_CAPABILITY_TASK_SYSTEM: &str = "task_system";
