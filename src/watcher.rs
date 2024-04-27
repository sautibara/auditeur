use crate::prelude::*;

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
