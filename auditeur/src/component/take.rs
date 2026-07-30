use std::marker::PhantomData;

use crate::{
    any,
    app::map::Dependency,
    component::{
        Component, ComponentIdPath, IdentifiedComponent, IdentifiedType, RegistryOf,
        registry::{ErasedLoader, Loaded, Loader, Registries, Registry},
    },
    input::CloneIn,
    prelude::*,
};

pub trait TakeMessage: IdentifiedType + any::AnyConst + Send + Sized + 'static {}

pub struct Giver<M: TakeMessage> {
    sender: Dependency<flume::Sender<M>>,
}

impl<M: TakeMessage> Giver<M> {
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

pub struct Taker<M: TakeMessage> {
    receiver: Dependency<flume::Receiver<M>>,
}

impl<M: TakeMessage> Taker<M> {
    pub async fn take(&mut self) -> Option<M> {
        self.receiver.recv_async().await.ok()
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

#[identified("auditeur:take_messages")]
pub struct TakeMessages;

impl Registry for TakeMessages {
    type Loader = ErasedLoader<TakeLoaded>;
    type Loaded = TakeLoaded;
}

impl RegistryOf for TakeMessages {
    type Registry = Registries;
}

impl<M: TakeMessage> Component<TakeMessages> for M {
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

pub struct AppTakeMut<'a> {
    app: &'a mut App,
}

impl<'a> AppTakeMut<'a> {
    pub(crate) const fn new(app: &'a mut App) -> Self {
        Self { app }
    }

    /// # Errors
    ///
    /// - Propagated from [`AppRegistriesMut::get`](crate::app::registry::AppRegistriesMut::get)
    pub async fn taker<M: TakeMessage>(self) -> Result<Taker<M>, Error> {
        self.app
            .components_mut()
            .get::<TakeMessages, _>(ComponentIdPath::Static(M::id()), async |loaded, _| {
                loaded.get_receiver()
            })
            .await
            .map(|receiver| Taker { receiver })
    }

    /// # Errors
    ///
    /// - Propagated from [`AppRegistriesMut::get`](crate::app::registry::AppRegistriesMut::get)
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
