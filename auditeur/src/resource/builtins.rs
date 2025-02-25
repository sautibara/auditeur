// use std::marker::PhantomData;
//
// use tracing::trace;
//
// use crate::prelude::*;
//
// /// A [`Listener`] that logs all messages for `M` in [`tracing`].
// #[derive(Create, Runnable)]
// #[listener("auditeur:logger")]
// pub struct Logger<M: Message + std::fmt::Debug> {
//     phantom_data: PhantomData<M>,
// }
//
// impl<M: Message + std::fmt::Debug> Default for Logger<M> {
//     fn default() -> Self {
//         Self {
//             phantom_data: PhantomData,
//         }
//     }
// }
//
// #[has_input]
// impl<M: Message + std::fmt::Debug> Listen<M> for Logger<M> {
//     async fn on(&mut self, message: M, (): ()) -> Result<()> {
//         trace!("Message: {message:?}");
//         Ok(())
//     }
// }

// #[async_trait]
// impl<M: Message + std::fmt::Debug> Listener<M> for Logger<M> {
//     type Input = app::EventReceiver;
//
//     async fn handle(&mut self, message: M, _: &mut app::EventReceiver) -> Result<()> {
//         trace!("Message: {:?}", message);
//         Ok(())
//     }
//
//     async fn registry_event(
//         &mut self,
//         event: RegistryEvent,
//         receiver: &mut app::EventReceiver,
//     ) -> Result<()> {
//         // NOTE: The event is already logged as a span
//         if event.is_error() {
//             error!("Received an error.");
//         } else {
//             debug!("Received an event.");
//         };
//         receiver.mark_seen();
//         Ok(())
//     }
//
//     async fn exit(&mut self, receiver: &mut app::EventReceiver) -> Result<()> {
//         while let Ok(event) = receiver.try_recv() {
//             debug!("RegistryEvent: {:?}", event);
//         }
//         debug!("Logger exiting... Messages after this point won't be logged");
//         Ok(())
//     }
// }
