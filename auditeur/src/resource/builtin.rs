use log::trace;

use crate::prelude::*;

#[resource("auditeur:logger")]
pub struct Logger;

#[async_trait]
impl<M: Message + std::fmt::Debug> Listener<M> for Logger {
    type Input = ();

    async fn handle(&mut self, message: M, (): &mut ()) -> anyhow::Result<()> {
        trace!("{:?}", message);
        Ok(())
    }
}
