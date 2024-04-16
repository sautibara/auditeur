use log::trace;

use crate::prelude::*;

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
