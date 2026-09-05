//! [`Component`]s that provide [broadcast](mod@broadcast)-style message streams to the app.
//!
//! # Examples
//!
//! ```
//! use auditeur::prelude::*;
//! # use auditeur::*;
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Error> {
//! #[listen_message("test:favorite_number")]
//! #[derive(Clone, PartialEq, Eq, Debug)]
//! struct FavoriteNumber(usize);
//!
//! let mut app = App::builder().with::<FavoriteNumber>(()).spawn();
//!
//! // Shouters are the sending halves of listen channels,
//! let shouter = app.listen_mut().shouter::<FavoriteNumber>().await?;
//!
//! // and they could send messages without blocking.
//! shouter.shout(FavoriteNumber(1));
//!
//! // Listeners are the receiving halves of the channel,
//! // and they can receive messages by waiting with a mutable reference.
//! let mut listener_1 = app.listen_mut().listener::<FavoriteNumber>().await?;
//! let mut listener_2 = app.listen_mut().listener::<FavoriteNumber>().await?;
//!
//! // They can only receive messages sent after they're obtained,
//! // so they won't receive the `FavoriteNumber(1)` message.
//! assert!(listener_1.is_empty() && listener_2.is_empty());
//!
//! // But now that they're listening, the shouter only has to send the message once.
//! shouter.shout(FavoriteNumber(2));
//!
//! // And every single listener gets a clone of the message.
//! assert_eq!(listener_1.listen().await, FavoriteNumber(2));
//! assert_eq!(listener_2.listen().await, FavoriteNumber(2));
//!
//! drop((listener_1, listener_2, shouter));
//! auditeur::test::finish(app, false).await
//! # }
//! ```
//!
//! ```
//! use auditeur::app::event::ComponentLifecycle;
//! use auditeur::component::bundle::Bundles;
//!
//! use auditeur::prelude::*;
//! # use auditeur::*;
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Error> {
//! let mut app = App::builder().spawn();
//!
//! // App events are a commonly used listen message sent by the app.
//! let mut listener = app.listen_mut().listener::<auditeur::app::Event>().await?;
//!
//! // They can be used to track the lifecycles of components and dependencies,
//! // errors occurring in the app, and a few more rare use-cases.
//!
//! // Here, we add a new, empty bundle to the app.
//! app.components_mut()
//!     .add(Children::new().with::<Bundle>(new_bundle!("test:bundle")))
//!     .await;
//!
//! loop {
//!     // Then, we start listening to app events.
//!     let event = listener.listen().await;
//!
//!     // And exit once we get the message for when the bundle was added.
//!     if let auditeur::app::Event::Component { id, lifecycle } = event
//!         && lifecycle == ComponentLifecycle::Add
//!         && id == ComponentId::inn::<Bundles>(ident!("test:bundle"))
//!     {
//!         break;
//!     }
//! }
//!
//! drop(listener);
//! auditeur::test::finish(app, false).await
//! # }
//! ```

use std::marker::PhantomData;

use tokio::sync::broadcast::{self, error::RecvError};

use crate::{
    any,
    app::event::Event,
    component::{
        ComponentIdPath, IdentifiedComponent, IdentifiedType, Register, RegistryOf,
        registry::{ErasedLoader, Loaded, Loader, Registries, Registry},
    },
    input::CloneIn,
    prelude::*,
};

/// [`Component`]s that provide [broadcast](mod@broadcast)-style message streams to the app.
///
/// This can be automatically implemented using the
/// [`#[listen_message]`](macro@crate::listen_message) macro.
///
/// See the [module documentation](self) for more information.
pub trait ListenMessage: IdentifiedType + Clone + any::AnyConst + Send + Sized + 'static {}

/// The sending half of a [`ListenMessage`] channel.
pub struct Shouter<M: ListenMessage> {
    sender: Dependency<broadcast::Sender<M>>,
}

impl<M: ListenMessage> Shouter<M> {
    /// Sends a new message to the channel.
    ///
    /// This message will be received by all [`Listener`]s of the channel that have already been
    /// created.
    pub fn shout(&self, message: M) {
        let _ = self.sender.send(message);
    }

    /// Obtains a [`Listener`] of the same channel, tracking the dependency in `app`.
    pub async fn subscribe(&self, app: &mut App) -> Listener<M> {
        Listener {
            receiver: Dependency::clone_with(&self.sender, app, async |inner, _| inner.subscribe())
                .await,
        }
    }
}

impl<M: ListenMessage> CloneIn for Shouter<M> {
    async fn clone_in(&self, app: &mut App) -> Self {
        Self {
            sender: Dependency::clone_with(&self.sender, app, async |inner, _| inner.clone()).await,
        }
    }
}

impl<M: ListenMessage> Input for Shouter<M> {
    async fn create(app: &mut App) -> Result<Self, Error> {
        app.listen_mut().shouter().await
    }
}

#[allow(clippy::used_underscore_items)]
mod shouter_send_sync_impls {
    use super::{ListenMessage, Shouter};
    use crate::app::Dependency;
    use tokio::sync::broadcast;

    // Automatically implement [`Send`] and [`Sync`] to bypass trait recursion limits.
    // SAFETY: Since M is [`Send`], [`broadcast::Sender`] is [`Send`] and [`Sync`].
    unsafe impl<M: ListenMessage> Send for Shouter<M> {}
    unsafe impl<M: ListenMessage> Sync for Shouter<M> {}

    const fn _check_bound<T: Sync>(_: &T) {}
    const fn _validate_fields<M: ListenMessage>(x: &Shouter<M>) {
        let Shouter { sender } = x;
        _check_bound::<Dependency<broadcast::Sender<M>>>(sender);
    }
}

/// The receiving half of a [`ListenMessage`] channel.
pub struct Listener<M: ListenMessage> {
    pub(crate) receiver: Dependency<broadcast::Receiver<M>>,
}

impl<M: ListenMessage> Listener<M> {
    /// Waits until a message is sent in the channel and returns it.
    #[allow(clippy::missing_panics_doc)]
    pub async fn listen(&mut self) -> M {
        loop {
            match self.receiver.recv().await {
                Ok(val) => return val,
                Err(RecvError::Closed) => unreachable!(
                    "channel shouldn't hang up because the loaded always keeps a sender alive"
                ),
                Err(RecvError::Lagged(_)) => (),
            }
        }
    }

    /// Returns `true` if there are no messages ready in the channel for this listener.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.receiver.is_empty()
    }
}

impl Listener<Event> {
    /// Releases this listener's dependency on the [`Event`] message stream.
    ///
    /// This is safe because the [`Event`] stream is created by the app and thus its unloading
    /// doesn't matter. This may be necessary if you want to listen to events while also waiting
    /// for the app to exit, as keeping the dependency alive would prevent the app from exiting.
    pub fn release_dependency(&mut self) {
        Dependency::release_unsafe(&mut self.receiver);
    }
}

impl<M: ListenMessage> CloneIn for Listener<M> {
    async fn clone_in(&self, app: &mut App) -> Self {
        Self {
            receiver: Dependency::clone_with(&self.receiver, app, async |inner, _| {
                inner.resubscribe()
            })
            .await,
        }
    }
}

impl<M: ListenMessage> Input for Listener<M> {
    async fn create(app: &mut App) -> Result<Self, Error> {
        app.listen_mut().listener().await
    }
}

#[allow(clippy::used_underscore_items)]
mod listener_send_sync_impls {
    use super::{ListenMessage, Listener};
    use crate::app::Dependency;
    use tokio::sync::broadcast;

    // Automatically implement [`Send`] and [`Sync`] to bypass trait recursion limits.
    // SAFETY: Since M is [`Send`], [`broadcast::Receiver`] is [`Send`] and [`Sync`].
    unsafe impl<M: ListenMessage> Send for Listener<M> {}
    unsafe impl<M: ListenMessage> Sync for Listener<M> {}

    const fn _check_bound<T: Sync>(_: &T) {}
    const fn _validate_fields<M: ListenMessage>(x: &Listener<M>) {
        let Listener { receiver } = x;
        _check_bound::<Dependency<broadcast::Receiver<M>>>(receiver);
    }
}

/// The [`Registry`] for [`ListenMessage`]s.
///
/// This is automatically added to every [`App`].
#[identified("auditeur:listen_messages")]
pub struct ListenMessages;

impl Registry for ListenMessages {
    type Loader = ErasedLoader<ListenLoaded>;
    type Loaded = ListenLoaded;
}

impl RegistryOf for ListenMessages {
    type Registry = Registries;
}

impl<M: ListenMessage> Register<M> for ListenMessages {
    type ComponentInput = ();
    fn register((): Self::ComponentInput) -> (ErasedLoader<ListenLoaded>, Children) {
        (ErasedLoader::new(ListenLoader::<M>::new()), Children::new())
    }
}

struct ListenLoader<M: ListenMessage> {
    pd: PhantomData<fn() -> M>,
}

impl<M: ListenMessage> ListenLoader<M> {
    const fn new() -> Self {
        Self { pd: PhantomData }
    }
}

impl<M: ListenMessage> Loader<ErasedLoader<ListenLoaded>> for ListenLoader<M> {
    type Loaded = ListenLoaded;
    async fn load(&self, _: &mut App) -> Result<Self::Loaded, Error> {
        Ok(ListenLoaded::new::<M>())
    }
}

impl<M: ListenMessage> IdentifiedComponent for ListenLoader<M> {
    fn component_id_dyn(&self) -> ComponentId {
        ComponentId::within::<ListenMessages, M>()
    }
}

pub(crate) struct PersistentChannelLoader<M: ListenMessage> {
    sender: broadcast::Sender<M>,
}

impl<M: ListenMessage> PersistentChannelLoader<M> {
    pub const fn new(sender: broadcast::Sender<M>) -> Self {
        Self { sender }
    }
}

impl<M: ListenMessage> Loader<ErasedLoader<ListenLoaded>> for PersistentChannelLoader<M> {
    type Loaded = ListenLoaded;
    async fn load(&self, _: &mut App) -> Result<Self::Loaded, Error> {
        Ok(ListenLoaded::bootstrap(self.sender.clone()))
    }
}

impl<M: ListenMessage> IdentifiedComponent for PersistentChannelLoader<M> {
    fn component_id_dyn(&self) -> ComponentId {
        ComponentId::within::<ListenMessages, M>()
    }
}

#[derive(Any)]
#[doc(hidden)]
pub struct ListenLoaded {
    sender: Box<dyn any::Any + Send + Sync>,
}

impl ListenLoaded {
    fn new<M: ListenMessage>() -> Self {
        Self::bootstrap::<M>(broadcast::Sender::new(1024))
    }

    pub(crate) fn bootstrap<M: ListenMessage>(sender: broadcast::Sender<M>) -> Self {
        Self {
            sender: Box::new(LoadedInner::<M> { sender }),
        }
    }
}

#[derive(Any)]
struct LoadedInner<M: ListenMessage> {
    sender: broadcast::Sender<M>,
}

impl ListenLoaded {
    fn downcast<M: ListenMessage>(&self) -> Result<&LoadedInner<M>, Error> {
        self.sender
            .downcast_ref()
            .ok_or_else(|| Error::DowncastFailed {
                id: ComponentId::within::<ListenMessages, M>(),
            })
    }

    fn get_sender<M: ListenMessage>(&self) -> Result<broadcast::Sender<M>, Error> {
        self.downcast().map(|loaded| loaded.sender.clone())
    }

    fn get_receiver<M: ListenMessage>(&self) -> Result<broadcast::Receiver<M>, Error> {
        self.downcast().map(|loaded| loaded.sender.subscribe())
    }
}

impl Loaded for ListenLoaded {}

/// Methods for [`App`] to manage [`ListenMessage`]s.
pub struct AppListenMut<'a> {
    app: &'a mut App,
}

impl<'a> AppListenMut<'a> {
    pub(crate) const fn new(app: &'a mut App) -> Self {
        Self { app }
    }

    /// Obtains a new [`Listener`] from the app.
    ///
    /// # Errors
    ///
    /// - Propagated from [`AppComponentsMut::get`](crate::app::AppComponentsMut::get)
    pub async fn listener<M: ListenMessage>(self) -> Result<Listener<M>, Error> {
        self.app
            .components_mut()
            .get::<ListenMessages, _>(ComponentIdPath::Static(M::id()), async |loaded, _| {
                loaded.get_receiver()
            })
            .await
            .map(|receiver| Listener { receiver })
    }

    /// Obtains a new [`Shouter`] from the app.
    ///
    /// # Errors
    ///
    /// - Propagated from [`AppComponentsMut::get`](crate::app::AppComponentsMut::get)
    pub async fn shouter<M: ListenMessage>(self) -> Result<Shouter<M>, Error> {
        self.app
            .components_mut()
            .get::<ListenMessages, _>(ComponentIdPath::Static(M::id()), async |loaded, _| {
                loaded.get_sender()
            })
            .await
            .map(|sender| Shouter { sender })
    }
}
