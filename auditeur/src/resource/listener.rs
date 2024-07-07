use crate::prelude::*;

use anyhow::Result;
use log::trace;

#[async_trait]
pub trait Listener<M: Message>: Resource + Send {
    type Input: Input<M>;

    /// Handle a `M` for any reason
    async fn handle(
        &mut self,
        message: M,
        input: &mut <Self::Input as Input<M>>::Output,
    ) -> Result<ControlFlow<()>>;

    async fn init(
        &mut self,
        _input: &mut <Self::Input as Input<M>>::Output,
    ) -> Result<ControlFlow<()>> {
        Ok(ControlFlow::Continue(()))
    }

    async fn exit(&mut self, _input: &mut <Self::Input as Input<M>>::Output) -> Result<()> {
        Ok(())
    }

    fn constraints(&self) -> Constraints {
        Self::Input::constraints()
    }
}

#[resource("auditeur:logger")]
pub struct Logger;

#[async_trait]
impl<M: Message + std::fmt::Debug> Listener<M> for Logger {
    type Input = ();

    async fn handle(&mut self, message: M, (): &mut ()) -> anyhow::Result<ControlFlow<()>> {
        trace!("{:?}", message);
        Ok(ControlFlow::Continue(()))
    }
}

// /// A way to queue messages for when the listeners are set up
// #[derive(Default)]
// #[resource("core:queuer")]
// pub struct Queuer(Vec<M>);
//
// impl Queuer {
//     #[must_use]
//     pub fn new() -> Self {
//         Self::default()
//     }
//
//     #[must_use]
//     pub fn queue(mut self, message: M) -> Self {
//         self.0.push(message);
//         self
//     }
// }
//
// #[async_trait]
// impl Listener for Queuer {
//     type Input = Sender<M>;
//
//     async fn handle(
//         &mut self,
//         message: M,
//         sender: &mut Sender<M>,
//     ) -> Result<ControlFlow<()>> {
//         let res = match message {
//             M::Startup => {
//                 // send all messages
//                 for message in self.0.drain(..) {
//                     sender.send(message)?;
//                 }
//                 // exit
//                 ControlFlow::Break(())
//             }
//             _ => ControlFlow::Continue(()),
//         };
//         Ok(res)
//     }
// }
