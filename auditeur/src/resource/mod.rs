use std::any::Any;

use crate::{app::RegistryEvent, prelude::*};

use anyhow::Result;

pub mod builtin;

pub trait Resource: Any + Send {
    fn ident(&self) -> NamespacedIdRef<'static>;
    fn const_ident() -> NamespacedIdRef<'static>
    where
        Self: Sized;
}

#[async_trait]
pub trait Listener<M: Message>: Resource + Send {
    type Input: Input<M>;

    async fn handle(
        &mut self,
        message: M,
        input: &mut <Self::Input as Input<M>>::Output,
    ) -> Result<()>;

    async fn init(&mut self, input: &mut <Self::Input as Input<M>>::Output) -> Result<()> {
        let _ = input;
        Ok(())
    }

    async fn exit(&mut self, input: &mut <Self::Input as Input<M>>::Output) -> Result<()> {
        let _ = input;
        Ok(())
    }

    async fn registry_event(
        &mut self,
        event: RegistryEvent,
        input: &mut <Self::Input as Input<M>>::Output,
    ) -> Result<()> {
        let _ = (event, input);
        Ok(())
    }

    fn constraints(&self) -> Constraints {
        Self::Input::constraints()
    }
}

#[async_trait]
pub trait Watcher<M: Message>: Resource + Send {
    type Input: Input<M>;

    async fn run(&mut self, input: &mut <Self::Input as Input<M>>::Output) -> anyhow::Result<()>;

    async fn exit(&mut self, _input: &mut <Self::Input as Input<M>>::Output) -> anyhow::Result<()> {
        Ok(())
    }

    fn constraints(&self) -> Constraints {
        Self::Input::constraints()
    }
}
