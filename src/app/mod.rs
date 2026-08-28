mod snapshot;
mod state;

pub(crate) use snapshot::enrich_with_connections;
pub use snapshot::{ConnectionRow, PolicyGroup, ProxyRow, Snapshot, TrafficRate, fetch_snapshot};
pub use state::{
    Action, App, Focus, Input, InputMode, Operation, OperationError, OperationSuccess, Page,
    ProfileOperation, ProfileOperationError, ProfileOperationSuccess, StatusKind, StatusLine,
};
