use futures::pin_mut;
use tokio::select;
use tracing::debug;

use crate::{
    app::{LoadedResource, Loader, Runner, channels::EventReceiver},
    prelude::*,
};

const fn then_continue(conditional: bool) -> ControlFlow<()> {
    if conditional {
        ControlFlow::Continue(())
    } else {
        ControlFlow::Break(())
    }
}

/// A [`Resource`] that runs its own method in the main async loop
///
/// The main use of `Watcher`s is that they can ask for [`Input`]s from the rest of the [`App`]
pub trait Watcher: Resource + Send {
    type Input: Input;

    fn run(&mut self, input: &mut Self::Input) -> impl Future<Output = anyhow::Result<()>> + Send;

    fn exit(&mut self, input: &mut Self::Input) -> impl Future<Output = anyhow::Result<()>> + Send {
        async {
            let _ = input;
            Ok(())
        }
    }

    #[must_use]
    fn constraints() -> Constraints {
        Constraints::empty()
    }

    /// Returns a set of channels that this watcher "owns".
    ///
    /// These channels will automatically be added to the [registry](Registry), but special care
    /// should be taken to make sure that a specific channel isn't added multiple times.
    #[must_use]
    fn channels() -> Channels {
        Channels::new()
    }
}

pub(crate) struct WatcherLoader<W: Watcher + Create> {
    external_input: W::ExternalInput,
}

impl<W: Watcher + Create> WatcherLoader<W> {
    pub const fn new(external_input: W::ExternalInput) -> Self {
        Self { external_input }
    }
}

#[async_trait]
impl<W: Watcher + Create> Loader for WatcherLoader<W> {
    async fn load(self: Box<Self>, registry: &Registry) -> Result<Box<dyn LoadedResource>, Error> {
        let ident = ComponentIdRef::Resource(W::const_ident());
        let registry_input = W::RegistryInput::create(registry, ident).await?;
        let watcher = W::create(registry_input, self.external_input)
            .await
            .map_err(|error| Error::FromResource {
                id: W::const_ident().into(),
                error,
            })?;
        let runner = LoadedWatcher::build(watcher, registry).await?;
        Ok(Box::new(runner))
    }

    fn constraints(&self) -> Constraints {
        W::RegistryInput::constraints()
            .extend(W::Input::constraints())
            .extend(W::constraints())
    }

    fn ident(&self) -> &NamespacedIdRef {
        W::const_ident()
    }

    fn channels(&self) -> Channels {
        W::channels()
    }
}

struct LoadedWatcher<W: Watcher> {
    inner: W,
    input: W::Input,
    event_receiver: EventReceiver,
    event_sender: EventSender,
    #[cfg(feature = "runtime-loading")]
    registry: Registry,
}

impl<W: Watcher> LoadedWatcher<W> {
    pub async fn build(watcher: W, registry: &Registry) -> Result<Self, Error> {
        Ok(Self {
            #[cfg(feature = "runtime-loading")]
            registry: registry.handle(),
            event_receiver: registry.event_receiver(),
            event_sender: registry.event_sender(),
            input: <W::Input as Input>::create(registry, ComponentIdRef::Resource(watcher.ident()))
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

impl<W: Watcher> LoadedResource for LoadedWatcher<W> {
    fn resource(&self) -> &dyn Resource {
        &self.inner
    }

    fn resource_mut(&mut self) -> &mut dyn Resource {
        &mut self.inner
    }

    fn runner(self: Box<Self>) -> Result<Box<dyn Runner>, Box<dyn LoadedResource>> {
        Ok(self)
    }
}

#[async_trait]
impl<W: Watcher> Runner for LoadedWatcher<W> {
    async fn run(mut self: Box<Self>) {
        // precompute get requests before running the watcher
        #[cfg(feature = "runtime-loading")]
        let ident = crate::app::ComponentIdRef::Resource(W::const_ident());
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

impl<W: Watcher> Identifiable for LoadedWatcher<W> {
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
