//! [`Component`]s that provide [mpmc](mod@flume)-style message streams to the app.
//!
//! ```
//! use auditeur::prelude::*;
//! # use auditeur::*;
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Error> {
//! #[take_message("test:print")]
//! struct Print(String);
//!
//! let mut app = App::builder().with::<Print>(()).spawn();
//!
//! // Givers are the sender half of take channels,
//! let giver = app.take_mut().giver::<Print>().await?;
//!
//! // and they can send messages without blocking.
//! giver.give(Print(String::from("Hello, ")));
//!
//! // Takers are the receiving half of take channels.
//! let mut taker_1 = app.take_mut().taker::<Print>().await?;
//! let mut taker_2 = app.take_mut().taker::<Print>().await?;
//!
//! // They can receive messages sent before they're obtained,
//! assert_eq!(taker_1.take().await.0, "Hello, ");
//!
//! // or after they're obtained.
//! giver.give(Print(String::from("world!")));
//! assert_eq!(taker_2.take().await.0, "world!");
//!
//! // But each message can only ever be received once, unlike a broadcast channel.
//! assert!(taker_1.is_empty() && taker_2.is_empty());
//!
//! drop((giver, taker_1, taker_2));
//! auditeur::test::finish(app, false).await
//! # }

use std::marker::PhantomData;

use crate::{
    any,
    component::{
        ComponentIdPath, IdentifiedComponent, IdentifiedType, Register, RegistryOf,
        registry::{ErasedLoader, Loaded, Loader, Registries, Registry},
    },
    input::CloneIn,
    prelude::*,
};

/// [`Component`]s that provide [mpmc](mod@flume)-style message streams to the app.
///
/// This can be automatically implemented using the
/// [`#[take_message]`](macro@crate::take_message) macro.
///
/// See the [module documentation](self) for more information.
pub trait TakeMessage: IdentifiedType + any::AnyConst + Send + Sized + 'static {}

/// The sending half of a [`TakeMessage`] channel.
pub struct Giver<M: TakeMessage> {
    sender: Dependency<flume::Sender<M>>,
}

impl<M: TakeMessage> Giver<M> {
    /// Sends a new message to the channel.
    ///
    /// This will be received by whichever [`Taker`] calls [`Taker::take`] first.
    pub fn give(&self, message: M) {
        let _ = self.sender.send(message);
    }
}

impl<M: TakeMessage> CloneIn for Giver<M> {
    async fn clone_in(&self, app: &mut App) -> Self {
        Self {
            sender: Dependency::clone_with(&self.sender, app, async |inner, _| inner.clone()).await,
        }
    }
}

impl<M: TakeMessage> Input for Giver<M> {
    async fn create(app: &mut App) -> Result<Self, Error> {
        app.take_mut().giver().await
    }
}

#[allow(clippy::used_underscore_items)]
mod giver_send_sync_impls {
    use super::{Giver, TakeMessage};
    use crate::app::Dependency;

    // Automatically implement [`Send`] and [`Sync`] to bypass trait recursion limits.
    // SAFETY: Since M is [`Send`], [`flume::Receiver`] is [`Send`] and [`Sync`].
    unsafe impl<M: TakeMessage> Send for Giver<M> {}
    unsafe impl<M: TakeMessage> Sync for Giver<M> {}

    const fn _check_bound<T: Sync>(_: &T) {}
    const fn _validate_fields<M: TakeMessage>(x: &Giver<M>) {
        let Giver { sender } = x;
        _check_bound::<Dependency<flume::Sender<M>>>(sender);
    }
}

/// The receiving half of a [`TakeMessage`] channel.
pub struct Taker<M: TakeMessage> {
    receiver: Dependency<flume::Receiver<M>>,
}

impl<M: TakeMessage> Taker<M> {
    /// Waits until a message is sent in the channel and returns it.
    #[allow(clippy::missing_panics_doc)]
    pub async fn take(&mut self) -> M {
        self.receiver
            .recv_async()
            .await
            .expect("channel shouldn't hang up because the loaded keeps a sender alive")
    }

    /// Returns `true` if there are no messages ready in the channel.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.receiver.is_empty()
    }
}

impl<M: TakeMessage> CloneIn for Taker<M> {
    async fn clone_in(&self, app: &mut App) -> Self {
        Self {
            receiver: Dependency::clone_with(&self.receiver, app, async |inner, _| inner.clone())
                .await,
        }
    }
}

impl<M: TakeMessage> Input for Taker<M> {
    async fn create(app: &mut App) -> Result<Self, Error> {
        app.take_mut().taker().await
    }
}

#[allow(clippy::used_underscore_items)]
mod taker_send_sync_impls {
    use super::{TakeMessage, Taker};
    use crate::app::Dependency;

    // Automatically implement [`Send`] and [`Sync`] to bypass trait recursion limits.
    // SAFETY: Since M is [`Send`], [`flume::Receiver`] is [`Send`] and [`Sync`].
    unsafe impl<M: TakeMessage> Send for Taker<M> {}
    unsafe impl<M: TakeMessage> Sync for Taker<M> {}

    const fn _check_bound<T: Sync>(_: &T) {}
    const fn _validate_fields<M: TakeMessage>(x: &Taker<M>) {
        let Taker { receiver } = x;
        _check_bound::<Dependency<flume::Receiver<M>>>(receiver);
    }
}

/// The [`Registry`] for [`TakeMessage`]s.
///
/// This is automatically added to every [`App`].
#[identified("auditeur:take_messages")]
pub struct TakeMessages;

impl Registry for TakeMessages {
    type Loader = ErasedLoader<TakeLoaded>;
    type Loaded = TakeLoaded;
}

impl RegistryOf for TakeMessages {
    type Registry = Registries;
}

impl<M: TakeMessage> Register<M> for TakeMessages {
    type ComponentInput = ();
    fn register((): Self::ComponentInput) -> (ErasedLoader<TakeLoaded>, Children) {
        (ErasedLoader::new(TakeLoader::<M>::new()), Children::new())
    }
}

struct TakeLoader<M: TakeMessage> {
    pd: PhantomData<fn() -> M>,
}

impl<M: TakeMessage> TakeLoader<M> {
    const fn new() -> Self {
        Self { pd: PhantomData }
    }
}

impl<M: TakeMessage> Loader<ErasedLoader<TakeLoaded>> for TakeLoader<M> {
    type Loaded = TakeLoaded;
    async fn load(&self, _: &mut App) -> Result<Self::Loaded, Error> {
        Ok(TakeLoaded::new::<M>())
    }
}

impl<M: TakeMessage> IdentifiedComponent for TakeLoader<M> {
    fn component_id_dyn(&self) -> ComponentId {
        ComponentId::within::<TakeMessages, M>()
    }
}

#[derive(Any)]
#[doc(hidden)]
pub struct TakeLoaded {
    sender: Box<dyn any::Any + Send + Sync>,
}

#[derive(Any)]
struct LoadedInner<M: TakeMessage> {
    sender: flume::Sender<M>,
    receiver: flume::Receiver<M>,
}

impl TakeLoaded {
    fn new<M: TakeMessage>() -> Self {
        let (sender, receiver) = flume::unbounded();
        Self {
            sender: Box::new(LoadedInner::<M> { sender, receiver }),
        }
    }

    fn downcast<M: TakeMessage>(&self) -> Result<&LoadedInner<M>, Error> {
        self.sender
            .downcast_ref()
            .ok_or_else(|| Error::DowncastFailed {
                id: ComponentId::within::<TakeMessages, M>(),
            })
    }

    fn get_sender<M: TakeMessage>(&self) -> Result<flume::Sender<M>, Error> {
        self.downcast().map(|loaded| loaded.sender.clone())
    }

    fn get_receiver<M: TakeMessage>(&self) -> Result<flume::Receiver<M>, Error> {
        self.downcast().map(|loaded| loaded.receiver.clone())
    }
}

impl Loaded for TakeLoaded {}

/// Methods for [`App`] to manage [`TakeMessage`]s.
pub struct AppTakeMut<'a> {
    app: &'a mut App,
}

impl<'a> AppTakeMut<'a> {
    pub(crate) const fn new(app: &'a mut App) -> Self {
        Self { app }
    }

    /// Obtains a new [`Taker`] from the app.
    ///
    /// # Errors
    ///
    /// - Propagated from [`AppComponentsMut::get`](crate::app::AppComponentsMut::get)
    pub async fn taker<M: TakeMessage>(self) -> Result<Taker<M>, Error> {
        self.app
            .components_mut()
            .get::<TakeMessages, _>(ComponentIdPath::Static(M::id()), async |loaded, _| {
                loaded.get_receiver()
            })
            .await
            .map(|receiver| Taker { receiver })
    }

    /// Obtains a new [`Giver`] from the app.
    ///
    /// # Errors
    ///
    /// - Propagated from [`AppComponentsMut::get`](crate::app::AppComponentsMut::get)
    pub async fn giver<M: TakeMessage>(self) -> Result<Giver<M>, Error> {
        self.app
            .components_mut()
            .get::<TakeMessages, _>(ComponentIdPath::Static(M::id()), async |loaded, _| {
                loaded.get_sender()
            })
            .await
            .map(|sender| Giver { sender })
    }
}
