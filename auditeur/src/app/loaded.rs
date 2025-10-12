use futures::pin_mut;
use tokio::select;
#[allow(clippy::wildcard_imports)] // dnc
use tracing::*;

use crate::prelude::*;

use crate::resource::Identifiable;
use crate::resource::Resource as ResourceTrait;
use crate::resource::Watcher as WatcherTrait;

use super::RegistryEvent;
use super::channels::{EventReceiver, EventSender};

const fn then_continue(conditional: bool) -> ControlFlow<()> {
    if conditional {
        ControlFlow::Continue(())
    } else {
        ControlFlow::Break(())
    }
}

/// A [resource](mod@crate::resource) that has already been loaded into a [`Registry`]
#[allow(clippy::module_name_repetitions)] // without Loaded, it would conflict with Resource
pub trait LoadedResource: Identifiable + Send + Sync {
    /// Optionally provide a [`Runner`] to run in the main loop
    ///
    /// This runner is joined alongside every other runner, if it is given
    ///
    /// # Errors
    ///
    /// - Returns [`Err(Self)`] if this resource doesn't provide a runner
    fn runner(self: Box<Self>) -> Result<Box<dyn Runner>, Box<dyn LoadedResource>>;
    /// Obtain a reference to the original resource within a trait object
    fn resource(&self) -> &dyn ResourceTrait;
    /// Obtain a reference to the original resource within a trait object
    fn resource_mut(&mut self) -> &mut dyn ResourceTrait;
}

/// An object that can run a simple async function, see [`LoadedResource::runner`]
#[async_trait]
pub trait Runner: Identifiable + Send {
    async fn run(self: Box<Self>);
}

pub struct Resource<R: ResourceTrait> {
    resource: R,
}

impl<R: ResourceTrait> Resource<R> {
    pub const fn new(resource: R) -> Self {
        Self { resource }
    }
}

impl<R: ResourceTrait> LoadedResource for Resource<R> {
    fn runner(self: Box<Self>) -> Result<Box<dyn Runner>, Box<dyn LoadedResource>> {
        Err(self)
    }

    fn resource(&self) -> &dyn ResourceTrait {
        &self.resource
    }

    fn resource_mut(&mut self) -> &mut dyn ResourceTrait {
        &mut self.resource
    }
}

impl<R: ResourceTrait> Identifiable for Resource<R> {
    fn ident(&self) -> &'static NamespacedIdRef {
        self.resource.ident()
    }

    fn const_ident() -> &'static NamespacedIdRef
    where
        Self: Sized,
    {
        R::const_ident()
    }
}

pub struct Watcher<W: WatcherTrait> {
    inner: W,
    input: W::Input,
    event_receiver: EventReceiver,
    event_sender: EventSender,
    #[cfg(feature = "runtime-loading")]
    registry: Registry,
}

impl<W: WatcherTrait> Watcher<W> {
    pub async fn build(watcher: W, registry: &Registry) -> Result<Self, Error> {
        Ok(Self {
            #[cfg(feature = "runtime-loading")]
            registry: registry.handle(),
            event_receiver: registry.event_receiver(),
            event_sender: registry.event_sender(),
            input: <W::Input as Input>::create(
                registry,
                super::event::ComponentIdRef::Resource(watcher.ident()),
            )
            .await?,
            inner: watcher,
        })
    }

    #[instrument(name = "handle_registry_event", level = "debug", skip_all, fields(?message))]
    async fn handle_meta(
        message: Option<RegistryEvent>,
        #[cfg(feature = "runtime-loading")] reference: Result<
            &dyn crate::input::get::ResourceRef,
            &Error,
        >,
    ) -> ControlFlow<()> {
        let Some(message) = message else {
            return ControlFlow::Break(());
        };
        let to_exit = match message {
            RegistryEvent::Stop(ref id) if id == W::const_ident() => true,
            RegistryEvent::Exit => true,
            #[cfg(feature = "runtime-loading")]
            RegistryEvent::Get(ref get_request) => {
                trace!("recieved get request");

                if get_request.id() == W::const_ident() {
                    #[allow(clippy::option_if_let_else)] // doesn't return anything
                    if let Ok(reference) = reference {
                        let ident = get_request.dependent();
                        get_request.send(reference.clone_as_boxed(ident)).await;
                    } else {
                        warn!("id matched GetRequest, but MaybeGetRef returned Err");
                        get_request.cancel().await;
                    }
                }

                false
            }
            _ => false,
        };
        then_continue(!to_exit)
    }
}

impl<W: WatcherTrait> LoadedResource for Watcher<W> {
    fn resource(&self) -> &dyn ResourceTrait {
        &self.inner
    }

    fn resource_mut(&mut self) -> &mut dyn ResourceTrait {
        &mut self.inner
    }

    fn runner(self: Box<Self>) -> Result<Box<dyn Runner>, Box<dyn LoadedResource>> {
        Ok(self)
    }
}

#[async_trait]
impl<W: WatcherTrait> Runner for Watcher<W> {
    async fn run(mut self: Box<Self>) {
        // precompute get requests before running the watcher
        #[cfg(feature = "runtime-loading")]
        let ident = crate::app::event::ComponentIdRef::Resource(W::const_ident());
        #[cfg(feature = "runtime-loading")]
        let reference = self.inner.maybe_get_ref(&self.registry, ident).await;

        {
            let mut meta_receiver = self.event_receiver.resubscribe();
            let run = self.inner.run(&mut self.input);
            pin_mut!(run);

            loop {
                select! {
                    biased;
                    message = meta_receiver.recv() => {
                        let res = Self::handle_meta(
                            message,
                            #[cfg(feature = "runtime-loading")] reference.as_deref(),
                        ).await;
                        if res.is_break() {
                            break;
                        }
                    }
                    res = &mut run => {
                        if let Err(err) = res {
                            self.event_sender.send_error(W::const_ident(), err);
                        }
                        break;
                    },
                }
            }
        }

        let res = self.inner.exit(&mut self.input).await;
        if let Err(err) = res {
            self.event_sender.send_error(W::const_ident(), err);
        }

        debug!("exit");
    }
}

impl<W: WatcherTrait> Identifiable for Watcher<W> {
    fn ident(&self) -> &'static NamespacedIdRef {
        self.inner.ident()
    }

    fn const_ident() -> &'static NamespacedIdRef
    where
        Self: Sized,
    {
        W::const_ident()
    }
}
