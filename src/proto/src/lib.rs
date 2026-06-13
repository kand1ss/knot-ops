pub mod v1 {
    tonic::include_proto!("knot.v1");
}
pub mod echo {
    tonic::include_proto!("echo.v1");
}

pub use v1::*;
