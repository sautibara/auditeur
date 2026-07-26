use std::{fmt::Debug, hash::Hash, sync::Arc};

use tokio::sync::oneshot;

use crate::prelude::*;

#[must_use]
pub struct AppFnComponentsMut;

pub struct AppComponentsMut<'a> {
    pub(super) app: &'a mut App,
}

impl<'a> AppComponentsMut<'a> {
    pub(super) const fn new(app: &'a mut App) -> Self {
        Self { app }
    }
}

pub(crate) fn waiter_channel() -> (Releaser, Waiter) {
    let (sender, receiver) = oneshot::channel();
    let sender = Arc::new(std::sync::Mutex::new(Some(sender)));
    (Releaser { sender }, Waiter { receiver })
}

pub struct Waiter {
    receiver: oneshot::Receiver<()>,
}

impl Waiter {
    pub async fn wait(self) {
        let _ = self.receiver.await;
    }

    #[must_use]
    pub fn is_ready(&self) -> bool {
        !self.receiver.is_empty()
    }
}

#[derive(Clone)]
pub struct Releaser {
    sender: Arc<std::sync::Mutex<Option<oneshot::Sender<()>>>>,
}

impl Releaser {
    pub(crate) fn release(&self) {
        if let Ok(mut opt) = self.sender.try_lock()
            && let Some(sender) = opt.take()
        {
            let _ = sender.send(());
        }
    }
}

impl Hash for Releaser {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        let pointer = Arc::as_ptr(&self.sender);
        pointer.hash(state);
    }
}

impl PartialEq for Releaser {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.sender, &other.sender)
    }
}

impl Eq for Releaser {}

impl Debug for Releaser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let is_some = self.sender.lock().is_ok_and(|opt| opt.is_some());
        if is_some {
            write!(f, "Some(...)")
        } else {
            write!(f, "None")
        }
    }
}
