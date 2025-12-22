pub use async_io;
pub use blocking;
pub use crossbeam;
pub use futures;
pub use parking_lot;
pub use sharded_slab;
pub use tokio;
pub use tracing;
pub use tracing_subscriber;

// FIXME: futures
// WARN: nix, rustix

parking_lot::expect_rubicon!();
tokio::expect_rubicon!();
tracing::expect_rubicon!();
tracing_subscriber::expect_rubicon!();
async_io::expect_rubicon!();
crossbeam::expect_rubicon!();
sharded_slab::expect_rubicon!();
blocking::expect_rubicon!();
futures::expect_rubicon!();
