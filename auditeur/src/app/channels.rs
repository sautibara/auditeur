use auditeur_macros::CloneAs;
use std::collections::{HashMap, HashSet};
use std::fmt::Debug;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc, watch};
use tracing::info_span;

use crate::app::event::Error;
use crate::app::resource_map::ResourceMapConnection;
use crate::input::dependency;
use crate::{Message, prelude::*};

/// The default buffer size for a [`Listen`] or [`Take`] channel.
// TODO: we could allow changing this at runtime, it wouldn't be terrible
// mutable globals aren't always bad, right? D:
pub const DEFAULT_CHANNEL_SIZE: usize = 1024;

// TODO: message dependents
// TODO: message parent bundles

/// A set of message channels that [`Listen`] or [`Take`] can use.
///
/// Each channel that's used by [`Listen`] or [`Take`] must be added to the registry before
/// use. For [`Take`], this is done automatically through the resource that takes those messages.
/// For [`Listen`], however, since there isn't a central resource that owns the messages, this can't
/// be done. Instead, channels must be declared in the parent bundle before use by using
/// [`Bundle::with_channels`].
///
/// # Examples
///
/// ```
/// use auditeur::prelude::*;
/// # use auditeur::*;
///
/// #[derive(Clone)]
/// #[message("test:added_message")]
/// struct AddedMessage;
///
/// #[derive(Clone)]
/// #[message("test:unadded_message")]
/// struct UnaddedMessage;
///
/// # #[tokio::main]
/// # fn main() {
///
/// let channels = Channels::new()
///     .with_listen_channel<AddedMessage>();
///
/// assert!(channels.listener<AddedMessage>().await.is_ok());
/// assert!(channels.listener<UnaddedMessage>().await.is_err());
///
///
/// // you can also add a [`Channels`] to a [`Bundle`]
/// let _ = new_bundle!("test:bundle").with_channels(channels);
/// # }
/// ```
// TODO: write doctest for adding the channels to a registry w/ resources using the channels
#[derive(Default, Clone)]
pub struct Channels {
    pub(crate) listen: HashMap<NamespacedId, AnyListenChannel>,
    pub(crate) take: HashMap<NamespacedId, AnyTakeChannel>,
}

impl Channels {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    // WARN: this return value does not have a dependency on it!
    pub(crate) fn add_listen_channel<M: ListenMessage>(&mut self) -> Shouter<M> {
        let sender = broadcast::Sender::new(DEFAULT_CHANNEL_SIZE);
        let shouter = Shouter::new(sender);
        let channel: ListenChannel<M> = shouter.clone_erased();
        self.listen
            .insert(M::const_ident().to_owned(), AnyListenChannel::new(channel));
        shouter
    }

    // WARN: this return value does not have a dependency on it!
    pub(crate) fn add_take_channel<M: Message>(&mut self) -> Giver<M> {
        let (sender, receiver) = mpsc::channel(DEFAULT_CHANNEL_SIZE);
        let giver = Giver::new(sender);
        let receiver_cell = Arc::new(tokio::sync::Mutex::new(Some(receiver)));
        let channel: TakeChannel<M> = (giver.clone_erased(), receiver_cell);
        self.take
            .insert(M::const_ident().to_owned(), AnyTakeChannel::new(channel));
        giver
    }

    #[must_use]
    pub fn with_listen_channel<M: ListenMessage>(mut self) -> Self {
        self.add_listen_channel::<M>();
        self
    }

    #[must_use]
    pub fn with_take_channel<M: Message>(mut self) -> Self {
        self.add_take_channel::<M>();
        self
    }

    pub(crate) fn listener<M: ListenMessage>(&self) -> Result<Listener<M>, crate::Error> {
        let any_channel = self
            .listen
            .get(M::const_ident())
            .ok_or_else(AnyListenChannel::error::<M>)?;
        any_channel.listener()
    }

    pub(crate) fn shouter<M: ListenMessage>(&self) -> Result<Shouter<M>, crate::Error> {
        let any_channel = self
            .listen
            .get(M::const_ident())
            .ok_or_else(AnyListenChannel::error::<M>)?;
        any_channel.shouter()
    }

    pub(crate) async fn taker<M: Message>(&self) -> Result<Taker<M>, crate::Error> {
        let any_channel = self
            .take
            .get(M::const_ident())
            .ok_or_else(AnyTakeChannel::error::<M>)?;
        any_channel.taker().await
    }

    pub(crate) fn giver<M: Message>(&self) -> Result<Giver<M>, crate::Error> {
        let any_channel = self
            .take
            .get(M::const_ident())
            .ok_or_else(AnyTakeChannel::error::<M>)?;
        any_channel.giver()
    }

    /// Adds all channels in `other` to `self`.
    ///
    /// This is equivalent to [`Self::add`], but takes `self` by ownership rather than by reference.
    pub fn add(&mut self, other: Self) {
        self.listen.extend(other.listen);
        self.take.extend(other.take);
    }

    /// Adds all channels in `other` to `self`.
    ///
    /// This is equivalent to [`Self::add`], but takes `self` by ownership rather than by reference.
    #[must_use]
    pub fn combine_with(mut self, other: Self) -> Self {
        self.add(other);
        self
    }

    pub(crate) fn ids(&self) -> HashSet<NamespacedId> {
        (self.listen.keys())
            .chain(self.take.keys())
            .cloned()
            .collect()
    }
}

impl Debug for Channels {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Channels")
            .field("listen", &self.listen.keys())
            .field("take", &self.take.keys())
            .finish()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ChannelsRef<'a> {
    inner: ChannelsRefEnum<'a>,
    event_sender: &'a EventSender,
}

#[derive(Debug, Clone, Copy)]
enum ChannelsRefEnum<'a> {
    Building(ChannelsRefBuilding<'a>),
    #[cfg(feature = "runtime-loading")]
    Running(ChannelsRefRunning<'a>),
}

impl<'a> ChannelsRef<'a> {
    #[must_use]
    pub(crate) const fn new_building(
        channels: &'a Channels,
        event_sender: &'a EventSender,
    ) -> Self {
        Self {
            inner: ChannelsRefEnum::Building(ChannelsRefBuilding { channels }),
            event_sender,
        }
    }

    #[must_use]
    pub(crate) const fn new_running(
        resource_map: &'a ResourceMapConnection,
        event_sender: &'a EventSender,
    ) -> Self {
        Self {
            inner: ChannelsRefEnum::Running(ChannelsRefRunning { resource_map }),
            event_sender,
        }
    }

    /// Returns the [`Listener`] channel with message `M`.
    ///
    /// This is the receiving half of a [`Listen`] channel.
    ///
    /// # Errors
    ///
    /// - [`crate::Error::ListenMessageNotFound`] if `M` hasn't been
    ///   [added](Channels::with_listen_channel) yet.
    pub async fn listener<M: ListenMessage>(
        &self,
        dependent: ComponentIdRef<'_>,
    ) -> Result<Listener<M>, crate::Error> {
        let channel = match &self.inner {
            ChannelsRefEnum::Building(building) => building.listener(),
            #[cfg(feature = "runtime-loading")]
            ChannelsRefEnum::Running(running) => running.listener().await,
        }?;
        Ok(channel.with_dependent(self.event_sender, dependent))
    }

    /// Returns the [`Shouter`] channel with message `M`.
    ///
    /// This is the sending half of a [`Listen`] channel.
    ///
    /// # Errors
    ///
    /// - [`crate::Error::ListenMessageNotFound`] if `M` hasn't been
    ///   [added](Channels::with_listen_channel) yet.
    pub async fn shouter<M: ListenMessage>(
        &self,
        dependent: ComponentIdRef<'_>,
    ) -> Result<Shouter<M>, crate::Error> {
        let channel = match &self.inner {
            ChannelsRefEnum::Building(building) => building.shouter(),
            #[cfg(feature = "runtime-loading")]
            ChannelsRefEnum::Running(running) => running.shouter().await,
        }?;
        Ok(channel.with_dependent(self.event_sender, dependent))
    }

    /// Takes the [`Taker`] channel with message `M`.
    ///
    /// This is the receiving half of a [`Take`] channel.
    ///
    /// Note that only one `Taker` can exist at a time in a `Take` channel, and this will error if
    /// multiple resources try to take a `Taker` for the same message.
    ///
    /// # Errors
    ///
    /// - [`crate::Error::TakeMessageNotFound`] if `M` hasn't been
    ///   [added](Channels::with_take_channel) yet.
    /// - [`crate::Error::TakeTakerTaken`] if the taker for `M` has already been taken.
    pub async fn taker<M: Message>(
        &self,
        dependent: ComponentIdRef<'_>,
    ) -> Result<Taker<M>, crate::Error> {
        let channel = match &self.inner {
            ChannelsRefEnum::Building(building) => building.taker().await,
            #[cfg(feature = "runtime-loading")]
            ChannelsRefEnum::Running(running) => running.taker().await,
        }?;
        Ok(channel.with_dependent(self.event_sender, dependent))
    }

    /// Returns the [`Giver`] channel with message `M`.
    ///
    /// This is the sending half of a [`Take`] channel.
    ///
    /// # Errors
    ///
    /// - [`crate::Error::TakeMessageNotFound`] if `M` hasn't been
    ///   [added](Channels::with_take_channel) yet.
    pub async fn giver<M: Message>(
        &self,
        dependent: ComponentIdRef<'_>,
    ) -> Result<Giver<M>, crate::Error> {
        let channel = match &self.inner {
            ChannelsRefEnum::Building(building) => building.giver(),
            #[cfg(feature = "runtime-loading")]
            ChannelsRefEnum::Running(running) => running.giver().await,
        }?;
        Ok(channel.with_dependent(self.event_sender, dependent))
    }
}

#[derive(Debug)]
pub struct ChannelsRefMut<'a> {
    inner: ChannelsRefMutEnum<'a>,
    event_sender: &'a EventSender,
}

#[derive(Debug)]
enum ChannelsRefMutEnum<'a> {
    Building(ChannelsRefMutBuilding<'a>),
    #[cfg(feature = "runtime-loading")]
    Running(ChannelsRefRunning<'a>),
}

impl<'a> ChannelsRefMut<'a> {
    #[must_use]
    pub(crate) const fn new_building(
        channels: &'a mut Channels,
        event_sender: &'a EventSender,
    ) -> Self {
        Self {
            inner: ChannelsRefMutEnum::Building(ChannelsRefMutBuilding { channels }),
            event_sender,
        }
    }

    #[must_use]
    pub(crate) const fn new_running(
        resource_map: &'a ResourceMapConnection,
        event_sender: &'a EventSender,
    ) -> Self {
        Self {
            inner: ChannelsRefMutEnum::Running(ChannelsRefRunning { resource_map }),
            event_sender,
        }
    }

    #[must_use]
    pub const fn imm(&self) -> ChannelsRef<'_> {
        match &self.inner {
            ChannelsRefMutEnum::Building(building) => ChannelsRef {
                inner: ChannelsRefEnum::Building(building.imm()),
                event_sender: self.event_sender,
            },
            ChannelsRefMutEnum::Running(running) => ChannelsRef {
                inner: ChannelsRefEnum::Running(*running),
                event_sender: self.event_sender,
            },
        }
    }

    /// Adds all channels in `other` to `self`.
    ///
    /// This is equivalent to [`Channels::combine_with`], but takes `self` by reference rather than
    /// by ownership.
    pub async fn add(&mut self, other: Channels) {
        match &mut self.inner {
            ChannelsRefMutEnum::Building(building) => building.add(other),
            #[cfg(feature = "runtime-loading")]
            ChannelsRefMutEnum::Running(running) => running.add(other).await,
        }
    }
}

/// A normal, owned [`Channels`] struct with no synchronization.
#[derive(Clone, Copy, Debug)]
struct ChannelsRefBuilding<'a> {
    channels: &'a Channels,
}

impl ChannelsRefBuilding<'_> {
    fn listener<M: ListenMessage>(self) -> Result<Listener<M>, crate::Error> {
        self.channels.listener()
    }

    fn shouter<M: ListenMessage>(self) -> Result<Shouter<M>, crate::Error> {
        self.channels.shouter()
    }

    async fn taker<M: Message>(self) -> Result<Taker<M>, crate::Error> {
        self.channels.taker().await
    }

    fn giver<M: Message>(self) -> Result<Giver<M>, crate::Error> {
        self.channels.giver()
    }
}

#[derive(Debug)]
struct ChannelsRefMutBuilding<'a> {
    channels: &'a mut Channels,
}

impl ChannelsRefMutBuilding<'_> {
    const fn imm(&self) -> ChannelsRefBuilding<'_> {
        ChannelsRefBuilding {
            channels: self.channels,
        }
    }

    fn add(&mut self, other: Channels) {
        self.channels.add(other);
    }
}

trait Channel: crate::Any + Send + Sync {
    fn clone_erased(&self) -> Box<dyn Channel>;
}

pub(crate) struct AnyChannel {
    boxed: Box<dyn Channel>,
}

impl AnyChannel {
    fn new<C: Channel>(channel: C) -> Self {
        Self {
            boxed: Box::new(channel),
        }
    }

    fn downcast_ref<T>(&self) -> Option<&T>
    where
        T: crate::Any,
    {
        (&*self.boxed as &dyn crate::Any).downcast_ref()
    }
}

impl Clone for AnyChannel {
    fn clone(&self) -> Self {
        Self {
            boxed: self.boxed.clone_erased(),
        }
    }
}

pub(crate) enum TypedAnyChannel {
    Listen(AnyListenChannel),
    Take(AnyTakeChannel),
}

impl TypedAnyChannel {
    pub(crate) const fn as_listen(&self) -> Option<&AnyListenChannel> {
        if let Self::Listen(v) = self {
            Some(v)
        } else {
            None
        }
    }

    pub(crate) const fn as_take(&self) -> Option<&AnyTakeChannel> {
        if let Self::Take(v) = self {
            Some(v)
        } else {
            None
        }
    }
}

impl Debug for TypedAnyChannel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Listen(_) => write!(f, "Listen(_)"),
            Self::Take(_) => write!(f, "Take(_)"),
        }
    }
}

type ListenChannel<M> = Shouter<M>;
#[derive(Clone)]
pub(crate) struct AnyListenChannel(AnyChannel);

impl AnyListenChannel {
    fn new<M: ListenMessage>(channel: ListenChannel<M>) -> Self {
        Self(AnyChannel::new(channel))
    }

    pub fn listener<M: ListenMessage>(&self) -> Result<Listener<M>, crate::Error> {
        self.listen_channel().map(Shouter::subscribe)
    }

    pub fn shouter<M: ListenMessage>(&self) -> Result<Shouter<M>, crate::Error> {
        self.listen_channel().map(Shouter::clone_erased)
    }

    fn listen_channel<M: ListenMessage>(&self) -> Result<&ListenChannel<M>, crate::Error> {
        self.0.downcast_ref().ok_or_else(Self::error::<M>)
    }

    pub fn error<M: ListenMessage>() -> crate::Error {
        crate::Error::ListenMessageNotFound {
            id: M::const_ident().to_owned(),
        }
    }
}

impl<M: ListenMessage> Channel for ListenChannel<M> {
    fn clone_erased(&self) -> Box<dyn Channel> {
        Box::new(self.clone_erased())
    }
}

type TakeReceiverCell<M> = Arc<tokio::sync::Mutex<Option<mpsc::Receiver<M>>>>;
type TakeChannel<M> = (Giver<M>, TakeReceiverCell<M>);
#[derive(Clone)]
pub(crate) struct AnyTakeChannel(AnyChannel);

impl AnyTakeChannel {
    fn new<M: Message>(channel: TakeChannel<M>) -> Self {
        Self(AnyChannel::new(channel))
    }

    async fn taker<M: Message>(&self) -> Result<Taker<M>, crate::Error> {
        let (_, receiver_cell_arc) = self.take_channel()?;
        let receiver_res = receiver_cell_arc.lock().await.take();
        let receiver = receiver_res.ok_or_else(|| crate::Error::TakeTakerTaken {
            id: M::const_ident().to_owned(),
        })?;
        Ok(Taker {
            receiver,
            source: receiver_cell_arc.clone(),
            _dependency: None,
        })
    }

    fn giver<M: Message>(&self) -> Result<Giver<M>, crate::Error> {
        let (giver, _) = self.take_channel()?;
        Ok(giver.clone_erased())
    }

    fn take_channel<M: Message>(&self) -> Result<&TakeChannel<M>, crate::Error> {
        self.0.downcast_ref().ok_or_else(Self::error::<M>)
    }

    pub fn error<M: Message>() -> crate::Error {
        crate::Error::TakeMessageNotFound {
            id: M::const_ident().to_owned(),
        }
    }
}

impl<M: Message> Channel for TakeChannel<M> {
    fn clone_erased(&self) -> Box<dyn Channel> {
        let (giver, cell) = self;
        Box::new((giver.clone_erased(), cell.clone()))
    }
}

/// A [`Channels`] struct that defers to [`ResourceMap`](crate::app::resource_map::ResourceMap) for
/// storing and obtaining channels.
#[cfg(feature = "runtime-loading")]
#[derive(Debug, Clone, Copy)]
pub struct ChannelsRefRunning<'a> {
    resource_map: &'a ResourceMapConnection,
}

#[cfg(feature = "runtime-loading")]
impl ChannelsRefRunning<'_> {
    async fn listener<M: ListenMessage>(&self) -> Result<Listener<M>, crate::Error> {
        self.resource_map
            .listen_channel(M::const_ident().to_owned())
            .await
            .ok_or_else(|| AnyListenChannel::error::<M>())?
            .listener()
    }

    async fn shouter<M: ListenMessage>(&self) -> Result<Shouter<M>, crate::Error> {
        self.resource_map
            .listen_channel(M::const_ident().to_owned())
            .await
            .ok_or_else(|| AnyListenChannel::error::<M>())?
            .shouter()
    }

    async fn taker<M: Message>(&self) -> Result<Taker<M>, crate::Error> {
        self.resource_map
            .take_channel(M::const_ident().to_owned())
            .await
            .ok_or_else(|| AnyTakeChannel::error::<M>())?
            .taker()
            .await
    }

    async fn giver<M: Message>(&self) -> Result<Giver<M>, crate::Error> {
        self.resource_map
            .take_channel(M::const_ident().to_owned())
            .await
            .ok_or_else(|| AnyTakeChannel::error::<M>())?
            .giver()
    }

    async fn add(&self, channels: Channels) {
        self.resource_map.add_channels(channels).await;
    }
}

fn dependency<M: Message>(
    sender: &EventSender,
    dependent: ComponentIdRef<'_>,
) -> dependency::Handle {
    dependency::Handle::new(
        sender.clone(),
        dependency::Full {
            link: dependency::Link {
                dependency: ComponentId::Message(M::const_ident().to_owned()),
                dependent: dependent.into_owned(),
            },
            removable: dependency::Removable::Inherit,
        },
    )
}

/// The sending half of a [`Listen`] channel.
///
/// All values sent by this channel will be received by all current [`Listener`] channels (of the
/// same message). However, [`Listener`]s created after a message is sent may not receive it
/// (history will not be kept).
///
/// This (currently) wraps a [`broadcast::Sender`].
#[derive(PathAny, CloneAs)]
pub struct Shouter<M: ListenMessage> {
    sender: broadcast::Sender<M>,
    _dependency: Option<dependency::Handle>,
}

impl<M: ListenMessage> Shouter<M> {
    pub(crate) const fn new(sender: broadcast::Sender<M>) -> Self {
        Self {
            sender,
            _dependency: None,
        }
    }

    pub fn send(&self, message: M) {
        let _ = self.sender.send(message);
    }

    #[must_use]
    fn subscribe(&self) -> Listener<M> {
        Listener {
            receiver: self.sender.subscribe(),
            _dependency: None,
        }
    }

    fn clone_erased(&self) -> Self {
        Self {
            sender: self.sender.clone(),
            _dependency: None,
        }
    }

    fn with_dependent(self, sender: &EventSender, dependent: ComponentIdRef<'_>) -> Self {
        Self {
            _dependency: Some(dependency::<M>(sender, dependent)),
            ..self
        }
    }
}

impl<M: ListenMessage> Input for Shouter<M> {
    async fn create(
        registry: &Registry,
        dependent: ComponentIdRef<'_>,
    ) -> Result<Self, crate::Error> {
        registry.channels().shouter(dependent).await
    }

    fn constraints() -> Constraints {
        Constraints::empty().after_message(M::const_ident())
    }
}

#[allow(clippy::used_underscore_binding)] // in debug impl
impl<M: ListenMessage> Debug for Shouter<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shouter")
            .field("sender", &self.sender)
            .field("_dependency", &self._dependency)
            .finish()
    }
}

/// The receiving half of a [`Listen`] channel.
///
/// This receives all messages sent by [`Shouter`]s (of the same message). However, if this is
/// created after a message was sent, this channel may not receive that message (history is not
/// kept).
///
/// This (currently) wraps a [`broadcast::Receiver`].
#[derive(PathAny)]
pub struct Listener<M: ListenMessage> {
    receiver: broadcast::Receiver<M>,
    _dependency: Option<dependency::Handle>,
}

impl<M: ListenMessage> Listener<M> {
    pub async fn recv(&mut self) -> Option<M> {
        self.receiver.recv().await.ok()
    }

    fn with_dependent(self, sender: &EventSender, dependent: ComponentIdRef<'_>) -> Self {
        Self {
            _dependency: Some(dependency::<M>(sender, dependent)),
            ..self
        }
    }
}

impl<M: ListenMessage> Input for Listener<M> {
    async fn create(
        registry: &Registry,
        dependent: ComponentIdRef<'_>,
    ) -> Result<Self, crate::Error> {
        registry.channels().listener(dependent).await
    }

    fn constraints() -> Constraints {
        Constraints::empty().after_message(M::const_ident())
    }
}

#[allow(clippy::used_underscore_binding)] // in debug impl
impl<M: ListenMessage> Debug for Listener<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Listener")
            .field("receiver", &self.receiver)
            .field("_dependency", &self._dependency)
            .finish()
    }
}

/// The sending half of a [`Take`] channel.
///
/// All values sent by this sender will be given directly to a single [`Taker`], if it exists.
/// However, if no `Taker` is currently running, the messages will be queued up to a certain
/// maximum length (see [`DEFAULT_CHANNEL_SIZE`]). If there is no capacity in the channel,
/// [`Self::give`] will await for space to be made.
///
/// This (currently) wraps a [`mpsc::Sender`].
#[derive(PathAny, CloneAs)]
pub struct Giver<M: Message> {
    sender: mpsc::Sender<M>,
    _dependency: Option<dependency::Handle>,
}

impl<M: Message> Giver<M> {
    pub(crate) const fn new(sender: mpsc::Sender<M>) -> Self {
        Self {
            sender,
            _dependency: None,
        }
    }

    pub async fn give(&self, message: M) {
        let _ = self.sender.send(message).await;
    }

    #[doc(hidden)]
    #[must_use]
    pub fn clone_erased(&self) -> Self {
        Self {
            sender: self.sender.clone(),
            _dependency: None,
        }
    }

    fn with_dependent(self, sender: &EventSender, dependent: ComponentIdRef<'_>) -> Self {
        Self {
            sender: self.sender,
            _dependency: Some(dependency::<M>(sender, dependent)),
        }
    }
}

#[allow(clippy::used_underscore_binding)] // in debug impl
impl<M: Message> Debug for Giver<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Giver")
            .field("sender", &self.sender)
            .field("_dependency", &self._dependency)
            .finish()
    }
}

impl<M: Message> Input for Giver<M> {
    async fn create(
        registry: &Registry,
        dependent: ComponentIdRef<'_>,
    ) -> Result<Self, crate::Error> {
        registry.channels().giver(dependent).await
    }

    fn constraints() -> Constraints {
        Constraints::empty().after_message(M::const_ident())
    }
}

/// The receiving half of a [`Take`] channel.
///
/// Only one [`Taker`] can exist at a time for a single message in the app. Once this is dropped,
/// the channel will be given back to the [`App`] so that a new resource could get a taker.
///
/// This (currently) wraps a [`mpsc::Receiver`].
#[derive(PathAny)]
pub struct Taker<M: Message> {
    receiver: mpsc::Receiver<M>,
    source: TakeReceiverCell<M>,
    _dependency: Option<dependency::Handle>,
}

impl<M: Message> Taker<M> {
    pub async fn recv(&mut self) -> Option<M> {
        self.receiver.recv().await
    }

    #[allow(clippy::used_underscore_binding)] // just to set
    fn with_dependent(mut self, sender: &EventSender, dependent: ComponentIdRef<'_>) -> Self {
        self._dependency = Some(dependency::<M>(sender, dependent));
        self
    }
}

impl<M: Message> Drop for Taker<M> {
    fn drop(&mut self) {
        let (_, dummy_receiver) = mpsc::channel(1);
        let receiver = std::mem::replace(&mut self.receiver, dummy_receiver);
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let source = self.source.clone();
            handle.spawn(async move {
                let mut source = source.lock().await;
                *source = Some(receiver);
            });
        } else {
            let mut source = self.source.blocking_lock();
            *source = Some(receiver);
        }
    }
}

#[allow(clippy::used_underscore_binding)] // in debug impl
impl<M: Message> Debug for Taker<M> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Taker")
            .field("receiver", &self.receiver)
            .field("source", &self.source)
            .field("_dependency", &self._dependency)
            .finish()
    }
}

impl<M: Message> Input for Taker<M> {
    async fn create(
        registry: &Registry,
        dependent: ComponentIdRef<'_>,
    ) -> Result<Self, crate::Error> {
        registry.channels().taker(dependent).await
    }

    fn constraints() -> Constraints {
        Constraints::empty().after_message(M::const_ident())
    }
}

/// A sender for [`RegistryEvent`]s
///
/// Each of these events will always be read by every other receiver; see [`EventReceiver`] for
/// more information about the channel.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "path-any", derive(PathAny))]
pub struct EventSender {
    watch: watch::Sender<Vec<RegistryEvent>>,
}

impl EventSender {
    pub(crate) fn new() -> Self {
        Self {
            watch: watch::Sender::new(Vec::new()),
        }
    }

    pub(crate) fn send(&self, event: RegistryEvent) {
        self.watch.send_modify(|vec| vec.push(event));
    }

    pub(crate) fn extend<I: IntoIterator<Item = RegistryEvent>>(&self, events: I) {
        self.watch.send_modify(|vec| vec.extend(events));
    }

    pub fn send_error(&self, ident: impl Into<NamespacedId>, error: anyhow::Error) {
        let error = Error::new(ident.into(), error);
        error!("{error}");
        self.send(RegistryEvent::Error(error));
    }

    pub fn handle_error<T>(&self, res: anyhow::Result<T>, id: &NamespacedIdRef) -> Option<T> {
        res.map_err(|err| self.send_error(id, err)).ok()
    }

    pub fn send_errors<E: Into<anyhow::Error>>(
        &self,
        ident: impl Into<NamespacedId>,
        errors: impl IntoIterator<Item = E>,
    ) {
        let ident = ident.into();
        let events = errors
            .into_iter()
            .map(|err| Error::new(ident.clone(), err.into()))
            .inspect(|val| error!("{val}"))
            .map(RegistryEvent::Error);
        self.extend(events);
    }

    pub(crate) fn subscribe(&self) -> EventReceiver {
        EventReceiver::new(self.watch.subscribe())
    }
}

impl Input for EventSender {
    async fn create(registry: &Registry, _: ComponentIdRef<'_>) -> Result<Self, crate::Error> {
        Ok(registry.event_sender())
    }
}

/// A receiver for [`RegistryEvent`]s
///
/// This will always hold the entire history of events, even ones that happened before the receiver
/// was created. This allows receivers to make models of the [`Registry`] using its entire state,
/// like [`ResourceMap`](crate::app::resource_map::ResourceMap).
#[derive(Debug)]
#[cfg_attr(feature = "path-any", derive(PathAny))]
pub struct EventReceiver {
    watch: watch::Receiver<Vec<RegistryEvent>>,
    index: usize,
}

impl EventReceiver {
    const fn new(receiver: watch::Receiver<Vec<RegistryEvent>>) -> Self {
        Self {
            watch: receiver,
            index: 0,
        }
    }

    /// Receive the oldest unhandled message or return [`None`] if the channel has closed.
    #[instrument(name = "recv_registry_event", skip(self))]
    pub async fn recv(&mut self) -> Option<RegistryEvent> {
        loop {
            let value = self.watch.borrow_and_update().get(self.index).cloned();
            if let Some(val) = value {
                self.index += 1;
                return Some(val);
            }
            self.watch
                .changed()
                .instrument(info_span!("waiting_for_new_event"))
                .await
                .ok()?;
        }
    }

    /// Receive an unhandled message if it is ready
    ///
    /// # Errors
    ///
    /// - [`EventTryRecvError::NotReady`] if the stream is not ready
    /// - [`EventTryRecvError::Closed`] if the stream is closed
    pub fn try_recv(&mut self) -> Result<RegistryEvent, EventTryRecvError> {
        let value = self.watch.borrow_and_update().get(self.index).cloned();
        if let Some(val) = value {
            self.index += 1;
            // can't use else 'cause the compiler gets angry at the mutable ref above
            return Ok(val);
        }
        // the easiest way to check if the channel is closed is through this method
        if self.watch.has_changed().is_err() {
            Err(EventTryRecvError::Closed)
        } else {
            Err(EventTryRecvError::NotReady)
        }
    }

    /// Resubscribe to the channel, starting from the beginning.
    #[must_use]
    pub fn resubscribe(&self) -> Self {
        Self {
            watch: self.watch.clone(),
            index: 0,
        }
    }

    /// Mark the most recent message as seen, ignoring recv
    pub fn mark_seen(&mut self) {
        let vec = self.watch.borrow_and_update();
        self.index = vec.len();
    }
}

impl Input for EventReceiver {
    async fn create(registry: &Registry, _: ComponentIdRef<'_>) -> Result<Self, crate::Error> {
        Ok(registry.event_receiver())
    }
}

/// The result for [`EventReceiver::try_recv`]
pub enum EventTryRecvError {
    Closed,
    NotReady,
}

impl EventTryRecvError {
    /// Returns `true` if the event try recv error is [`Closed`].
    ///
    /// [`Closed`]: EventTryRecvError::Closed
    #[must_use]
    pub const fn is_closed(&self) -> bool {
        matches!(self, Self::Closed)
    }

    /// Returns `true` if the event try recv error is [`NotReady`].
    ///
    /// [`NotReady`]: EventTryRecvError::NotReady
    #[must_use]
    pub const fn is_not_ready(&self) -> bool {
        matches!(self, Self::NotReady)
    }
}

/// A handle on a [`Registry`] that can be used to stop one single resource or exit the entire
/// program
#[derive(Clone, Debug)]
#[cfg_attr(feature = "path-any", derive(PathAny))]
pub struct Stopper {
    sender: EventSender,
}

impl Stopper {
    pub(super) const fn new(sender: EventSender) -> Self {
        Self { sender }
    }

    /// Tell all resources in the [`Registry`] to exit
    pub fn exit(&self) {
        self.sender.send(RegistryEvent::Exit);
    }

    /// Stop the resource `R`, if it exists
    #[inline]
    pub fn stop<R: Identifiable>(&self) {
        self.stop_id(R::const_ident());
    }

    /// Stop the resource with id `id`, if it exists
    pub fn stop_id(&self, id: &NamespacedIdRef) {
        let id = id.to_owned();
        self.sender.send(RegistryEvent::Stop(id));
    }

    pub fn remove_dependency(&self, link: dependency::Link) {
        self.sender.send(RegistryEvent::RemoveDependency(link));
    }
}

impl Input for Stopper {
    async fn create(registry: &Registry, _: ComponentIdRef<'_>) -> Result<Self, crate::Error> {
        Ok(registry.stopper())
    }
}

// TODO: ErrorSink struct, like Stopper
