use crate::{
    component::resource::{Resource, ResourceParts, Runner},
    prelude::*,
};

pub struct PureMethod;
impl<R: Resource> ResourceParts<R> for PureMethod {
    type Runner = PureRunner;
    async fn runner(_: R, _: &mut App) -> Result<Self::Runner, Error> {
        Ok(PureRunner {})
    }
}

pub struct PureRunner {}
impl Runner for PureRunner {
    async fn run(self, _: &mut App) {}
}
