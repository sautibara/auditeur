// use tracing::{debug, error, trace};
//
// use crate::{app, prelude::*};
//
// #[derive(Default, Create)]
// #[resource("auditeur:logger")]
// pub struct Logger<M: Message + std::fmt::Debug>;
//
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
