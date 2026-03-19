use strum::{Display, EnumString};

#[derive(Display, EnumString, Debug, PartialEq)]
pub enum ServiceStatus {
    Stopped,
    Starting,
    Running,
    Failed
}