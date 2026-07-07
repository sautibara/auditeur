use std::marker::PhantomData;

use tokio::sync::broadcast;

use crate::{
    app::{
        map::Dependency,
        registry::{ErasedLoader, Loaded, Loader, Registries, Registry},
    },
    component::{Component, ComponentIdPath, IdentifiedComponent, IdentifiedType, RegistryOf},
    input::CloneIn,
    prelude::*,
};

pub trait ListenMessage:
    IdentifiedType + Clone + crate::AnyConst + Send + Sync + Sized + 'static
{
}

pub struct Shouter<M: ListenMessage> {
    sender: Dependency<broadcast::Sender<M>>,
}

impl<M: ListenMessage> Shouter<M> {
    pub fn shout(&self, message: M) {
        let _ = self.sender.send(message);
    }

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

pub struct Listener<M: ListenMessage> {
    pub(crate) receiver: Dependency<broadcast::Receiver<M>>,
}

impl<M: ListenMessage> Listener<M> {
    pub async fn listen(&mut self) -> Option<M> {
        self.receiver.recv().await.ok()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.receiver.is_empty()
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

#[identified("auditeur:listen_messages")]
pub struct ListenMessages;

impl Registry for ListenMessages {
    type Loader = ErasedLoader<ListenLoaded>;
    type Loaded = ListenLoaded;
}

impl RegistryOf for ListenMessages {
    type Registry = Registries;
}

impl<M: ListenMessage> Component<ListenMessages> for M {
    type ComponentInput = ();
    fn register((): Self::ComponentInput) -> (ErasedLoader<ListenLoaded>, Children) {
        (ErasedLoader::new(ListenLoader::<M>::new()), Children::new())
    }
}

struct ListenLoader<M: ListenMessage> {
    pd: PhantomData<M>,
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
pub struct ListenLoaded {
    sender: Box<dyn crate::Any + Send + Sync>,
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

pub struct AppListenMut<'a> {
    app: &'a mut App,
}

impl<'a> AppListenMut<'a> {
    pub(crate) const fn new(app: &'a mut App) -> Self {
        Self { app }
    }

    /// # Errors
    ///
    /// - Propagated from [`AppRegistriesMut::get`](crate::app::registry::AppRegistriesMut::get)
    pub async fn listener<M: ListenMessage>(self) -> Result<Listener<M>, Error> {
        self.app
            .registries_mut()
            .get::<ListenMessages, _>(ComponentIdPath::Static(M::id()), async |loaded, _| {
                loaded.get_receiver()
            })
            .await
            .map(|receiver| Listener { receiver })
    }

    /// # Errors
    ///
    /// - Propagated from [`AppRegistriesMut::get`](crate::app::registry::AppRegistriesMut::get)
    pub async fn shouter<M: ListenMessage>(self) -> Result<Shouter<M>, Error> {
        self.app
            .registries_mut()
            .get::<ListenMessages, _>(ComponentIdPath::Static(M::id()), async |loaded, _| {
                loaded.get_sender()
            })
            .await
            .map(|sender| Shouter { sender })
    }
}
