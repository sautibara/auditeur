use crate::{app::RegistryEvent, prelude::*};

use anyhow::Result;
use auditeur_macros::cfg_supertrait;

pub mod builtin;

pub trait Resource: Send + Sync + 'static {
    fn ident(&self) -> NamespacedIdRef<'static>;
    fn const_ident() -> NamespacedIdRef<'static>
    where
        Self: Sized;
}

// TODO: find a better name for this
#[allow(clippy::module_name_repetitions)]
#[cfg_supertrait(feature = "runtime-loading", crate::input::MaybeGetRef)]
pub trait DynamicResource: Resource + crate::Any {}

#[cfg(feature = "runtime-loading")]
impl<T: Resource + crate::Any + crate::input::MaybeGetRef> DynamicResource for T {}
#[cfg(not(feature = "runtime-loading"))]
impl<T: Resource + crate::Any> DynamicResource for T {}

#[async_trait]
pub trait Listener<M: Message>: DynamicResource + Send {
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
pub trait Watcher<M: Message>: DynamicResource + Send {
    type Input: Input<M>;

    async fn run(&mut self, input: &mut <Self::Input as Input<M>>::Output) -> anyhow::Result<()>;

    async fn exit(&mut self, _input: &mut <Self::Input as Input<M>>::Output) -> anyhow::Result<()> {
        Ok(())
    }

    fn constraints(&self) -> Constraints {
        Self::Input::constraints()
    }
}
