use crate::prelude::*;

#[async_trait]
pub trait Watcher<M: Message>: Resource + Send {
    type Input: Input<M>;

    async fn run(&mut self, input: &mut <Self::Input as Input<M>>::Output) -> anyhow::Result<()>;

    fn constraints(&self) -> Constraints {
        Self::Input::constraints()
    }
}
