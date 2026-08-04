//! A [`Resource`] that runs nothing in the async executor.
//!
//! Use [`#[pure_resource]`](macro@crate::pure_resource) to create one of these resources.

use crate::{
    component::resource::{Resource, ResourceParts, Runner},
    prelude::*,
};

/// A [`GetRunnerWith`](super::GetRunnerWith) [`Method`](super::GetRunnerWith::Method) that
/// provides an empty [`Runner`] who does nothing and immediately exits.
pub struct PureMethod;
impl<R: Resource> ResourceParts<R> for PureMethod {
    type Runner = PureRunner;
    async fn runner(_: R, _: &mut App) -> Result<Self::Runner, Error> {
        Ok(PureRunner {})
    }

    const OPTIONS: super::Options = super::Options::new().weak(true);
}

/// The [`Runner`] for [`PureMethod`].
pub struct PureRunner {}
impl Runner for PureRunner {
    async fn run(self, _: &mut App) {}
}
