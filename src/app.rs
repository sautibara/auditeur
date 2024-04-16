use std::any::Any;

use log::{debug, error};
use tokio::sync::broadcast::Receiver;

use crate::component::listener;
use crate::prelude::*;

pub struct App<M> {
    sender: Sender<M>,
    listeners: Vec<Box<dyn listener::SimpleObject<M>>>,
}

impl<M: Message> App<M> {
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        let sender = Sender::new(capacity);
        Self {
            sender,
            listeners: vec![],
        }
    }

    /// # Errors
    ///
    /// - If the plugin's input had an error when initializing
    pub fn register(
        &mut self,
        listener: impl Listener<M> + Send + 'static,
    ) -> Result<&mut Self, InputError> {
        let simple = listener::Simple::create(listener, self)?;
        self.listeners.push(simple);
        Ok(self)
    }

    #[must_use]
    pub fn sender(&self) -> Sender<M> {
        self.sender.clone()
    }

    /// Listen to all messages until [`M::Exit`] is sent
    pub async fn listen(self) {
        let sender = self.sender;
        let futures: Vec<_> = self
            .listeners
            .into_iter()
            .map(|listener| Self::listen_single(listener, sender.subscribe()))
            .collect(); // collect early so all recievers subscibe now
        futures::future::join_all(futures).await;
    }

    /// Listen to only a single `listener`
    async fn listen_single(
        mut listener: Box<dyn listener::SimpleObject<M>>,
        mut reciever: Receiver<M>,
    ) {
        let name = listener.ident();
        // send startup message
        let res = Self::handle(&mut listener, M::startup()).await;
        // some listeners only run on startup and immediately break (Queuer)
        if res.is_break() {
            return;
        }
        // listen to other messages
        while let Ok(message) = reciever.recv().await {
            // check if the message is an exit message
            // if it is, then the listener will exit after handling it
            let to_exit = message.is_exit();
            let res = Self::handle(&mut listener, message).await;
            if to_exit || res.is_break() {
                break;
            }
        }
        debug!("{name} exited");
    }

    /// Handle a single `message` with a `listener`
    async fn handle(
        listener: &mut Box<dyn listener::SimpleObject<M>>,
        message: M,
    ) -> ControlFlow<()> {
        listener.handle(message).await.unwrap_or_else(|err| {
            error!("{} failed to handle message: {err}", listener.ident());
            ControlFlow::Continue(())
        })
    }

    #[must_use]
    pub fn get<T: Listener<M> + Clone>(&self) -> Option<T> {
        self.listeners.iter().find_map(|listener| {
            // for some reason rust doesn't recognize that listener is an Any unless this is done
            let any: &dyn Any = listener;
            any.downcast_ref::<listener::Simple<M, T>>()
                .map(listener::Simple::listener)
                .cloned()
        })
    }
}
