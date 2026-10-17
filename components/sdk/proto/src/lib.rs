pub mod v1 {
    tonic::include_proto!("knot.v1");

    pub mod config {
        tonic::include_proto!("knot.v1.config");
    }

    pub mod task {
        tonic::include_proto!("knot.v1.task");
    }

    pub mod commands {
        tonic::include_proto!("knot.v1.commands");
    }

    pub mod data {
        tonic::include_proto!("knot.v1.data");
    }
}
