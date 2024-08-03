use crate::{app::RegistryEvent, prelude::*};

use anyhow::Result;
use auditeur_macros::cfg_supertrait;

pub mod builtin;

// #[cfg_supertrait(feature = "path-any", crate::path_any::PathAny, std::any::Any)]
pub trait Resource: Send + 'static {
    fn ident(&self) -> NamespacedIdRef<'static>;
    fn const_ident() -> NamespacedIdRef<'static>
    where
        Self: Sized;
}

#[cfg_supertrait(feature = "path-any", crate::path_any::PathAny, std::any::Any)]
// TODO: find a better name for this
#[allow(clippy::module_name_repetitions)]
pub trait DynamicResource: Resource {}

#[cfg(feature = "path-any")]
impl<T: Resource + crate::path_any::PathAny> DynamicResource for T {}
#[cfg(not(feature = "path-any"))]
impl<T: Resource + std::any::Any> DynamicResource for T {}

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
