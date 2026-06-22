use crate::{
    component::resource::{Resource, ResourceParts, Runner},
    prelude::*,
};

/// A [`Resource`] that runs its own method in the main async loop.
///
/// WARNING: There is no custom logic to tell the watcher to stop. Instead, watchers are expected
/// to carry around a [`StopListener`](super::StopListener) to know when to stop.
pub trait Watcher: Resource {
    fn run(&mut self) -> impl Future<Output = anyhow::Result<()>> + Send + Sync;

    #[must_use]
    fn children() -> Children {
        Children::new()
    }
}

pub struct WatcherMethod;
impl<W: Watcher> ResourceParts<W> for WatcherMethod {
    type Runner = WatcherRunner<W>;

    async fn runner(watcher: W, _: &mut App) -> Result<Self::Runner, Error> {
        Ok(WatcherRunner { watcher })
    }
}

pub struct WatcherRunner<W: Watcher> {
    watcher: W,
}

impl<W: Watcher> Runner for WatcherRunner<W> {
    async fn run(mut self, app: &mut App) {
        let res = self.watcher.run().await;
        app.errors_mut().drain_any_result(res).await;
    }
}
