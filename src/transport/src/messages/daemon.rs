use serde::{Serialize, Deserialize};
use crate::messages::Message;
use knot_core::data::ServiceData;
use knot_core::states::ServiceStatus;
use knot_core::utils::TimestampUtils;


#[derive(Serialize, Deserialize, Debug)]
pub struct ServiceStatusResponse {
    pid: u32,
    name: String,
    status: String,
    uptime: String,
    healthy: bool,
}
impl From<&ServiceData> for ServiceStatusResponse {
    fn from(s: &ServiceData) -> Self {
        Self {
            pid: s.pid,
            name: s.name.clone(),
            status: s.status.to_string(),
            uptime: TimestampUtils::format_uptime(s.timestamp),
            healthy: s.status == ServiceStatus::Running,
        }
    }
}


#[derive(Serialize, Deserialize, Debug)]
pub enum DaemonRequest {
    Down,
    Status
}

#[derive(Serialize, Deserialize, Debug)]
pub enum DaemonResponse {
    Ok,
    Error { message: String },
    Status { services: Vec<ServiceStatusResponse> }
}

pub type DaemonMessage = Message<DaemonRequest, DaemonResponse>;