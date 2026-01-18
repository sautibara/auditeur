use async_recursion::async_recursion;

use super::Loader;
use std::{
    collections::{HashMap, HashSet, hash_map},
    fmt::Debug,
    ops::Deref,
    sync::Arc,
};

use crate::{
    app::{ResourceMap, channels::TypedAnyChannel, event, resource_map},
    bundle,
    input::Constraint,
    prelude::*,
};

/// The main entry-point for the application: the thing that runs resources
///
/// The app handles loading [resources](crate::resource::Resource),
/// [listeners](Listener), and [watchers](Watcher),
/// giving them the correct [inputs](crate::input::Input) (including
/// [each other](crate::input::get::Get)), and [running them](Self::run) asynchronously. It also
/// makes sure they are loaded in the correct order, through [constraints](Constraints), allowing
/// them to have other dependencies.
///
/// The main container for resources is the [`Bundle`], and all resources that are added to an
/// `App` must be contained within one. As such, all resources added to an app must be added
/// through a bundle. If there are conflicts between resources with the same id, the one that is
/// added first is kept.
#[must_use = "An app does nothing unless it is run"]
pub struct App {
    loaders: HashMap<NamespacedId, Option<BundledLoader>>,
    bundles: HashMap<NamespacedId, Option<BuiltBundle>>,
    channels: HashMap<NamespacedId, Option<BuiltChannel>>,

    loading_loaders: HashMap<NamespacedId, NamespacedId>,
    // TODO: check if adding App events is a good idea
    events: Vec<RegistryEvent>,
}

struct BundledLoader {
    loader: Box<dyn Loader>,
    bundle: NamespacedId,
}

pub struct BuiltBundle {
    pub constraints: Constraints,
    pub parent: Option<NamespacedId>,
    pub resources: Arc<[NamespacedId]>,
    pub messages: Arc<[NamespacedId]>,
    pub bundles: Arc<[NamespacedId]>,
    #[cfg(feature = "runtime-loading")]
    pub init: Option<bundle::SharedInitFn>,
}

#[derive(Debug)]
struct BuiltChannel {
    channel: TypedAnyChannel,
    bundle: NamespacedId,
}

impl App {
    /// Create a new app
    pub fn new() -> Self {
        Self {
            loaders: HashMap::new(),
            bundles: HashMap::new(),
            channels: HashMap::new(),

            loading_loaders: HashMap::new(),
            events: Vec::new(),
        }
    }

    /// Add a [`Bundle`] to the app, supplied by a [`Supplier`](bundle::Supplier)
    ///
    /// This takes ownership of the [`App`], allowing for usage closer to a builder pattern
    pub fn with_bundle(mut self, bundle: impl bundle::Supplier) -> Self {
        self.add_bundle(bundle);
        self
    }

    /// Add a [`Bundle`] to the app, supplied by a [`Supplier`](bundle::Supplier)
    pub fn add_bundle(&mut self, bundle: impl bundle::Supplier) -> &mut Self {
        let bundle = bundle.build();
        bundle.apply_to(self, None);
        self
    }

    pub(crate) fn add_bundle_inner(&mut self, id: NamespacedId, bundle: BuiltBundle) {
        self.bundles.insert(id, Some(bundle));
    }

    pub(crate) fn add_loader(
        &mut self,
        loader: Box<dyn Loader>,
        bundle: &NamespacedIdRef,
    ) -> NamespacedId {
        self.add_channels(loader.channels(), bundle);

        let id = loader.ident().to_owned();
        // prefer the eldest loader
        self.loaders
            .entry(id.clone())
            .or_insert(Some(BundledLoader {
                loader,
                bundle: bundle.to_owned(),
            }));

        id
    }

    pub(crate) fn add_channels(&mut self, channels: Channels, bundle: &NamespacedIdRef) {
        let Channels { listen, take } = channels;

        let listen = listen
            .into_iter()
            .map(|(id, channel)| (id, TypedAnyChannel::Listen(channel)));
        let take = take
            .into_iter()
            .map(|(id, channel)| (id, TypedAnyChannel::Take(channel)));

        for (id, channel) in listen.chain(take) {
            let entry = self.channels.entry(id);
            match entry {
                hash_map::Entry::Occupied(entry) => {
                    error!(
                        "message '{}' added multiple times, using old message",
                        entry.key()
                    );
                }
                hash_map::Entry::Vacant(entry) => {
                    entry.insert(Some(BuiltChannel {
                        channel,
                        bundle: bundle.to_owned(),
                    }));
                }
            }
        }
    }

    /// [Build](Self::build) the `App` and run all [runners](super::Runner) to completion
    ///
    /// # Errors
    ///
    /// - If one of the loaders failed to build
    /// - If a dependency wasn't found
    pub async fn run(self) -> Result<(), (Self, Error)> {
        let registry = self.build().await?;
        registry.run().await;
        Ok(())
    }

    /// Runs an asynchronous function alongside the app, stopping the app once the function exits.
    /// The function may also take in [`Input`]s.
    ///
    /// # Errors
    ///
    /// - If one of the loaders failed to build
    /// - If a dependency wasn't found
    pub async fn run_with<F, Fut, I>(self, func: F) -> Result<(), Error>
    where
        F: FnOnce(I) -> Fut,
        Fut: Future<Output = ()>,
        I: Input,
    {
        let ident = ComponentIdRef::Resource(ident!("auditeur:run_with"));
        let registry = self.build().await.map_err(|(_, err)| err)?;
        let input = I::create(&registry, ident).await?;
        let stopper = Stopper::create(&registry, ident).await?;
        let running = tokio::spawn(registry.run());
        func(input).await;
        stopper.exit();
        let _ = running.await;
        Ok(())
    }

    /// Build the `App` into a [`Registry`], without running it
    ///
    /// # Errors
    ///
    /// - If one of the loaders failed to build
    /// - If a dependency wasn't found
    pub async fn build(self) -> Result<Registry, (Self, Error)> {
        let mut registry = Registry::new();
        self.add_to_registry(&mut registry).await?;
        Ok(registry)
    }

    /// Add the `App` into a [`Registry`], without running it
    ///
    /// # Errors
    ///
    /// - If one of the loaders failed to build
    /// - If a dependency wasn't found
    pub async fn add_to_registry(mut self, registry: &mut Registry) -> Result<(), (Self, Error)> {
        trace!("adding App to registry: {self:?}");
        self.add_to_registry_fallible(registry)
            .await
            .map_err(|err| (self, err))
    }

    async fn add_to_registry_fallible(&mut self, registry: &mut Registry) -> Result<(), Error> {
        // free events :)
        registry.event_sender_ref().extend(self.events.drain(..));
        // add the loaders and bundles that have already been loaded in the registry to our list
        // this is necessary so that constraints can know that their dependencies have already been
        // loaded
        self.loaders.extend(
            registry
                .loaded_resources()
                .await
                .into_iter()
                .map(|id| (id, None)),
        );
        self.bundles.extend(
            registry
                .loaded_bundles()
                .await
                .into_iter()
                .map(|id| (id, None)),
        );
        self.channels.extend(
            registry
                .loaded_messages()
                .await
                .into_iter()
                .map(|id| (id, None)),
        );

        if !self.loaders.contains_key(ResourceMap::const_ident()) {
            self.add_bundle(resource_map::init);
        }

        // the collect is needed so that the immutable reference to the keys doesn't mess with the
        // mutable references used by resolving
        #[allow(clippy::needless_collect)]
        for loader in self.loaders.keys().cloned().collect::<Vec<_>>() {
            self.resolve_loader(registry, &loader).await?;
        }
        Ok(())
    }

    #[async_recursion]
    async fn resolve_loader(
        &mut self,
        registry: &mut Registry,
        id: &NamespacedIdRef,
    ) -> Result<(), Error> {
        trace!("resolving loader {id}");
        let loader = self
            .loaders
            .get_mut(id)
            .ok_or_else(|| Error::LoaderNotFound { id: id.to_owned() })?;
        // if it's already finished, then early return
        let Some(BundledLoader { loader, bundle }) = loader.take() else {
            return Ok(());
        };
        // show that this loader is currently being loaded
        // TODO: use this to prevent dependency loops
        self.loading_loaders.insert(id.to_owned(), bundle.clone());
        // bundles always have to be resolved first
        // this doesn't cause a loop because resolve_bundle already [`Option::take`]s the bundle
        // out of the list, so it can't be loaded again
        self.resolve_bundle(registry, &bundle).await?;
        // then now that we have the parent bundle we could say that this resource is loading
        registry.event_sender_ref().send(RegistryEvent::Resolving {
            id: loader.ident().into(),
            bundle: bundle.clone(),
        });
        // if there are constraints, we must load them first
        self.resolve_constraints(registry, &loader.constraints())
            .await?;
        // then we can finally load the loader
        registry.add_loader(loader, bundle).await?;
        self.loading_loaders.remove(id);
        trace!("resolved loader {id}");
        Ok(())
    }

    #[async_recursion]
    async fn resolve_bundle(
        &mut self,
        registry: &mut Registry,
        id: &NamespacedIdRef,
    ) -> Result<(), Error> {
        trace!("resolving bundle {id}");
        let bundle = self
            .bundles
            .get_mut(id)
            .ok_or_else(|| Error::BundleNotFound { id: id.to_owned() })?;
        // if it's already finished, then early return
        let Some(bundle) = bundle.take() else {
            return Ok(());
        };
        // parent bundles always have to be resolved first
        if let Some(parent) = &bundle.parent {
            self.resolve_bundle(registry, parent).await?;
        }
        // then now that we have the parent bundle we could say that this bundle is loading
        registry
            .event_sender_ref()
            .send(RegistryEvent::BundleResolving {
                id: id.to_owned(),
                parent: bundle.parent.clone(),
                resources: bundle.resources.clone(),
                messages: bundle.messages.clone(),
                bundles: bundle.bundles.clone(),
            });
        // then the constraints also have to be resolved
        self.resolve_constraints(registry, &bundle.constraints)
            .await?;
        // then the bundle can finally be added to the registry
        registry.add_bundle(id.to_owned(), bundle);
        trace!("resolved bundle {id}");
        Ok(())
    }

    #[async_recursion]
    async fn resolve_message(
        &mut self,
        registry: &mut Registry,
        id: &NamespacedIdRef,
    ) -> Result<(), Error> {
        trace!("resolving message {id}");
        let channel = self
            .channels
            .get_mut(id)
            .ok_or_else(|| Error::MessageNotFound { id: id.to_owned() })?;
        // if it's already finished, then early return
        let Some(channel) = channel.take() else {
            return Ok(());
        };
        // the parent bundle always have to be resolved first
        self.resolve_bundle(registry, &channel.bundle).await?;

        match channel.channel {
            TypedAnyChannel::Listen(listen_channel) => {
                // then now that we have the parent bundle we could say that this bundle is loading
                registry
                    .event_sender_ref()
                    .send(RegistryEvent::ListenMessageLoaded {
                        id: id.to_owned(),
                        bundle: channel.bundle.clone(),
                    });

                // then the channel can finally be added to the registry
                let mut channels = Channels::new();
                channels.listen.insert(id.to_owned(), listen_channel);
                registry.channels_mut().add(channels).await;
            }
            TypedAnyChannel::Take(take_channel) => {
                registry
                    .event_sender_ref()
                    .send(RegistryEvent::TakeMessageLoaded {
                        id: id.to_owned(),
                        bundle: channel.bundle.clone(),
                    });

                let mut channels = Channels::new();
                channels.take.insert(id.to_owned(), take_channel);
                registry.channels_mut().add(channels).await;
            }
        }
        trace!("resolved message {id}");
        Ok(())
    }

    async fn resolve_constraints(
        &mut self,
        registry: &mut Registry,
        constraints: &Constraints,
    ) -> Result<(), Error> {
        for constraint in constraints {
            match constraint {
                Constraint::After(component) => match component {
                    ComponentId::Resource(id) => self.resolve_loader(registry, id).await?,
                    ComponentId::Bundle(id) => self.resolve_bundle(registry, id).await?,
                    ComponentId::Message(id) => self.resolve_message(registry, id).await?,
                },
            }
        }
        Ok(())
    }

    pub fn add_error(&mut self, source: NamespacedId, error: anyhow::Error) {
        self.events
            .push(RegistryEvent::Error(event::Error::new(source, error)));
    }

    pub fn add_errors<I: IntoIterator<Item = anyhow::Error>>(
        &mut self,
        source: &NamespacedIdRef,
        errors: I,
    ) {
        self.events.extend(
            errors
                .into_iter()
                .map(|error| RegistryEvent::Error(event::Error::new(source.to_owned(), error))),
        );
    }

    // TODO: documentation to clarify name
    pub fn unloaded_loaders(&self) -> impl Iterator<Item = &NamespacedIdRef> {
        self.loading_loaders.keys().map(Deref::deref)
    }

    #[must_use]
    pub fn unloaded_bundles(&self) -> HashSet<&NamespacedIdRef> {
        self.loading_loaders.values().map(Deref::deref).collect()
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl Debug for App {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("App")
            .field("bundles", &self.bundles.keys())
            .field("loaders", &self.loaders.keys())
            .field("channels", &self.channels)
            .field("events", &self.events)
            .field("loading_loaders", &self.loading_loaders)
            .finish()
    }
}
