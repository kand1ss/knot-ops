use crate::states::ServiceStatus;


pub struct ServiceData {
    pub pid: u32,
    pub name: String,
    pub status: ServiceStatus,
    pub timestamp: u64,
}