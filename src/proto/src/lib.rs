pub mod v1 {
    tonic::include_proto!("knot.v1");
}
pub mod echo {
    tonic::include_proto!("echo");
}

pub use v1::*;
