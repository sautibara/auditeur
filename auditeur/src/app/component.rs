use crate::prelude::*;

/// Return type of [`AppFn::components_mut`](super::AppFn::components_mut),
/// see [`AppComponentsMut`].
#[must_use]
pub struct AppFnComponentsMut;

pub struct AppComponentsMut<'a> {
    pub(super) app: &'a mut App,
}

impl<'a> AppComponentsMut<'a> {
    pub(super) const fn new(app: &'a mut App) -> Self {
        Self { app }
    }
}
