use crate::config::ServiceConfig;


pub enum ExitReason {
    UserRequest,
    Faulted
}

pub enum ProcessEvent<'a> {
    Started { pid: &'a str,  service: &'a ServiceConfig },
    Stopped { pid: &'a str, service: &'a ServiceConfig },
    Failed { pid: &'a str, service: &'a ServiceConfig },
    Exited { pid: &'a str, service: &'a ServiceConfig, reason: ExitReason }
}