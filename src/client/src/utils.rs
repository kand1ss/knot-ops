use std::time::Duration;
use tonic::Request;

pub fn request<R>(req: R, timeout: Option<Duration>) -> Request<R> {
    let mut request = Request::new(req);
    if let Some(duration) = timeout {
        request.set_timeout(duration);
    }
    request
}