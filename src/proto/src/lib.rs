pub mod config {
    pub mod v1 {
        tonic::include_proto!("knot.config.v1");
    }
}

pub mod command {
    pub mod v1 {
        tonic::include_proto!("knot.command.v1");
    }
}

pub mod commands {
    pub mod v1 {
        tonic::include_proto!("knot.commands.v1");
    }
}

pub mod data {
    pub mod v1 {
        tonic::include_proto!("knot.data.v1");
    }
}

pub mod api {
    pub mod v1 {
        tonic::include_proto!("knot.api.v1");
    }
}
