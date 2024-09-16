use log::trace;

use crate::{app, prelude::*};

#[derive(Default, Create)]
#[resource("auditeur:logger")]
pub struct Logger;

#[async_trait]
impl<M: Message + std::fmt::Debug> Listener<M> for Logger {
    type Input = app::EventReceiver;

    async fn handle(&mut self, message: M, _: &mut app::EventReceiver) -> Result<()> {
        trace!("Message: {:?}", message);
        Ok(())
    }

    async fn registry_event(
        &mut self,
        event: RegistryEvent,
        receiver: &mut app::EventReceiver,
    ) -> Result<()> {
        trace!("RegistryEvent: {:?}", event);
        receiver.mark_seen();
        Ok(())
    }

    async fn exit(&mut self, receiver: &mut app::EventReceiver) -> Result<()> {
        while let Ok(event) = receiver.try_recv() {
            trace!("RegistryEvent: {:?}", event);
        }
        trace!("Logger exiting... Messages after this point won't be logged");
        Ok(())
    }
}
