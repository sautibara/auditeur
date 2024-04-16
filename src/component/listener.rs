use std::any::Any;

use crate::prelude::*;

use super::input::Input;

use anyhow::Result;

#[async_trait]
pub trait Listener<M>: Resource + Send + Any {
    type Input: Input<M>;

    /// Handle a `M` for any reason
    async fn handle(
        &mut self,
        message: M,
        input: &mut <Self::Input as Input<M>>::Output,
    ) -> Result<ControlFlow<()>>;

    fn constraints(&self) -> Constraints {
        Constraints::empty()
    }
}

pub(crate) struct Simple<M, L: Listener<M>> {
    listener: L,
    input: <L::Input as Input<M>>::Output,
}

impl<M: Message, L: Listener<M>> Simple<M, L> {
    pub fn create(listener: L, app: &mut App<M>) -> Result<Box<dyn SimpleObject<M>>, InputError> {
        let input = <L::Input as Input<M>>::create(app)?;
        Ok(Box::new(Self { listener, input }))
    }

    pub const fn listener(&self) -> &L {
        &self.listener
    }
}

impl<M: Message, L: Listener<M>> Resource for Simple<M, L> {
    fn ident(&self) -> NamespacedIdRef<'static> {
        self.listener.ident()
    }

    fn const_ident() -> NamespacedIdRef<'static>
    where
        Self: Sized,
    {
        L::const_ident()
    }
}

#[async_trait]
pub(crate) trait SimpleObject<M>: Resource + Send + Any {
    /// Handle a `M` for any reason
    async fn handle(&mut self, message: M) -> Result<ControlFlow<()>>;

    fn constraints(&self) -> Constraints {
        Constraints::empty()
    }
}

#[async_trait]
impl<M: Message, L: Listener<M>> SimpleObject<M> for Simple<M, L> {
    async fn handle(&mut self, message: M) -> Result<ControlFlow<()>> {
        self.listener.handle(message, &mut self.input).await
    }

    fn constraints(&self) -> Constraints {
        self.listener
            .constraints()
            .extend(<L::Input as Input<M>>::constraints())
    }
}
