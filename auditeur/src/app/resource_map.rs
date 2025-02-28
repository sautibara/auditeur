//! A dependency tree of the [resources](mod@crate::resource) and [bundles](crate::Bundle) that are
//! currently loaded

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, Weak},
};

use anyhow::{anyhow, bail};
use dashmap::DashMap;
use tracing::trace;

use crate::{app::EventSender, prelude::*, resource::listener};

use super::{EventReceiver, GetRequest};

struct LoadedResource {
    id: NamespacedId,
    #[cfg(feature = "runtime-loading")]
    resource: tokio::sync::RwLock<Option<Box<dyn super::LoadedResource>>>,
    // hold a strong reference to the parent bundle to keep it alive
    _bundle: Arc<LoadedBundle>,
    // and strong references to all of its dependencies for the same reason
    dependencies: Vec<Arc<LoadedResource>>,
    sender: EventSender,
}

impl Drop for LoadedResource {
    fn drop(&mut self) {
        self.sender.send(RegistryEvent::Unloaded(self.id.clone()));
    }
}

#[cfg(feature = "runtime-loading")]
impl LoadedResource {
    async fn supply_owned(&self, resource: Box<dyn super::LoadedResource>) {
        trace!(
            "adding {} to the ResourceMap as a NonRunnable",
            resource.ident()
        );
        let mut resource_option = self.resource.write().await;
        *resource_option = Some(resource);
    }

    /// Runs `func` on the contained owned resource, if it exists
    async fn check_get_request(&self, get_request: &GetRequest) {
        let resource_option = self.resource.read().await;
        if let Some(resource) = resource_option.as_deref() {
            get_request.check_ref(resource.resource()).await;
        }
    }
}

struct LoadedBundle {
    id: NamespacedId,
    sender: EventSender,
    // hold a strong reference to a potential parent bundle to keep it alive
    _parent: Option<Arc<LoadedBundle>>,
}

impl Drop for LoadedBundle {
    fn drop(&mut self) {
        self.sender
            .send(RegistryEvent::BundleUnloaded(self.id.clone()));
    }
}

/// A dependency tree of the [resources](mod@crate::resource) and [bundles](crate::Bundle) that are
/// currently loaded
///
/// This tree is necessary for telling when [resources](RegistryEvent::Unloaded) or
/// [bundles](RegistryEvent::BundleUnloaded) are unloaded. While running resources can be
/// [stopped](RegistryEvent::Stopped), they only unload when every reference to them also unloads.
/// As such, the map keeps track of these [references](RegistryEvent::Reference)
///
/// Similarly, bundles only unload when all of its child resources and bundles also unload
///
/// This listener is automatically loaded to every [`App`](crate::app::App) that is constructed
/// when the `resource-map` feature is specified.
// The way that the map works is that each resource and bundle is stored in an Arc. When resources
// or bundles are dropped, they send messages to the Registry that they get unloaded, so that their
// lifetimes become the lifetime of what they represent. As such, the underlying functionality of
// the Arc is leveraged to count our references for us. To provide a basis for ownership, the map
// owns a list of the currently running resources which become the roots of what remains loaded - a
// running resource never unloads. Then, each shared resource holds a strong reference to each of
// its dependencies and its parent bundle, keeping them all alive while it's loaded. Bundles, too,
// keep their parents alive, if they exist. From this, dependencies are always kept alive at least
// as long as their dependents.
// TODO: add methods to actually view the entire map
#[derive(Runnable)]
#[listener(
    "auditeur:resource_map",
    registry_component = false,
    components((component = MainComponent, input = ())),
)]
pub struct ResourceMap {
    sender: EventSender,
    /// This contains all of the shared information
    shared: Arc<Shared>,
    /// The resources that either aren't loaded or have no references yet
    ///
    /// These can't be put in the main resources map, as they'd need a reference to keep them alive
    resource_queue: HashMap<NamespacedId, LoadedResource>,
    /// The bundles that either aren't loaded or have no references yet
    ///
    /// These can't be put in the main bundles map, as they'd need a reference to keep them alive
    bundle_queue: HashMap<NamespacedId, LoadedBundle>,
}

struct Shared {
    /// A map of all of the currently running resources
    ///
    /// These resources are the roots of what stays loaded, and as such, are stored as strong
    /// references
    runners: DashMap<NamespacedId, Arc<LoadedResource>>,
    /// A map of all of the resources that have ever been loaded
    ///
    /// If the underlying Arc unloads (no strong references point to it anymore), that means that
    /// the resource also must be unloaded. As such, this map holds no strong references to any of
    /// these resources, allowing them to unload if necessary.
    resources: DashMap<NamespacedId, Weak<LoadedResource>>,
    /// A map of all of the bundles that have ever been loaded
    bundles: DashMap<NamespacedId, Weak<LoadedBundle>>,
}

#[create]
impl Create for ResourceMap {
    async fn create(sender: EventSender) -> Result<Self> {
        Ok(Self {
            sender,
            shared: Arc::new(Shared {
                runners: DashMap::new(),
                resources: DashMap::new(),
                bundles: DashMap::new(),
            }),
            resource_queue: HashMap::new(),
            bundle_queue: HashMap::new(),
        })
    }
}

impl ResourceMap {
    /// Obtain a [`Handle`] to the map
    #[must_use]
    pub fn handle(&self) -> Handle {
        Handle {
            shared: self.shared.clone(),
        }
    }

    fn add_resource(&mut self, id: NamespacedId, parent: &NamespacedIdRef) -> Result<()> {
        let parent = self.clone_bundle(parent)?;
        self.resource_queue.insert(
            id.clone(),
            LoadedResource {
                id,
                #[cfg(feature = "runtime-loading")]
                resource: tokio::sync::RwLock::new(None),
                _bundle: parent,
                dependencies: Vec::new(),
                sender: self.sender.clone(),
            },
        );
        Ok(())
    }

    fn add_bundle(&mut self, id: NamespacedId, parent: Option<&NamespacedIdRef>) -> Result<()> {
        let parent = parent.map(|parent| self.clone_bundle(parent)).transpose()?;
        self.bundle_queue.insert(
            id.clone(),
            LoadedBundle {
                id,
                _parent: parent,
                sender: self.sender.clone(),
            },
        );
        Ok(())
    }

    fn clone_resource(&mut self, id: &NamespacedIdRef) -> Result<Arc<LoadedResource>> {
        if let Some((id, resource)) = self.resource_queue.remove_entry(id) {
            let resource = Arc::new(resource);
            self.shared.resources.insert(id, Arc::downgrade(&resource));
            Ok(resource)
        } else if let Some(resource) = self.get_resource(id) {
            Ok(resource)
        } else {
            bail!("could not find resource: {id}");
        }
    }

    fn clone_bundle(&mut self, id: &NamespacedIdRef) -> Result<Arc<LoadedBundle>> {
        if let Some((id, bundle)) = self.bundle_queue.remove_entry(id) {
            let bundle = Arc::new(bundle);
            self.shared.bundles.insert(id, Arc::downgrade(&bundle));
            Ok(bundle)
        } else if let Some(bundle) = self.get_bundle(id) {
            Ok(bundle)
        } else {
            bail!("could not find bundle: {id}");
        }
    }

    fn add_reference(&mut self, by: &NamespacedIdRef, to: &NamespacedIdRef) -> Result<()> {
        let to = self.clone_resource(to)?;
        let by = self
            .resource_queue
            .get_mut(by)
            .ok_or_else(|| anyhow!("could not find resource: {by}"))?;
        by.dependencies.push(to);
        Ok(())
    }

    fn get_resource(&self, id: &NamespacedIdRef) -> Option<Arc<LoadedResource>> {
        self.shared
            .resources
            .get(id)
            .and_then(|resource| resource.upgrade())
    }

    fn get_bundle(&self, id: &NamespacedIdRef) -> Option<Arc<LoadedBundle>> {
        self.shared
            .bundles
            .get(id)
            .and_then(|resource| resource.upgrade())
    }
}

#[doc(hidden)]
pub struct MainComponent {
    receiver: EventReceiver,
    should_exit: bool,
}

impl Input for MainComponent {
    async fn create(registry: &Registry, _: &NamespacedIdRef) -> Result<Self, Error> {
        Ok(Self {
            receiver: registry.event_receiver(),
            should_exit: false,
        })
    }
}

impl listener::Component<ResourceMap> for MainComponent {
    type Value = RegistryEvent;
    type UserInput<'s> = ();

    async fn wait(&mut self, _: &ResourceMap) -> anyhow::Result<ControlFlow<(), Self::Value>> {
        let res = self.receiver.recv().await;
        Ok(res.map_or(ControlFlow::Break(()), ControlFlow::Continue))
    }

    async fn handle(
        &mut self,
        event: RegistryEvent,
        map: &mut ResourceMap,
        (): Self::UserInput<'_>,
    ) -> anyhow::Result<ControlFlow<()>> {
        match event {
            RegistryEvent::BundleLoading { id, parent, .. } => {
                map.add_bundle(id, parent.as_deref())?;
            }
            RegistryEvent::Loading { id, bundle } => {
                map.add_resource(id, &bundle)?;
            }
            RegistryEvent::Reference { by, to } => {
                map.add_reference(&by, &to)?;
            }
            RegistryEvent::Running(id) => {
                let resource = map.clone_resource(&id)?;
                map.shared.runners.insert(id, resource);
            }
            RegistryEvent::Stopped(id) => {
                map.shared.runners.remove(&id);
                // this should be the last runner
                if self.should_exit && map.shared.runners.len() <= 1 {
                    return Ok(ControlFlow::Break(()));
                }
            }
            #[cfg(feature = "runtime-loading")]
            RegistryEvent::NonRunnable(supplier) => {
                if let Some(resource) = map.get_resource(supplier.id()) {
                    if let Some(owned) = supplier.take() {
                        resource.supply_owned(owned).await;
                    }
                }
            }
            #[cfg(feature = "runtime-loading")]
            RegistryEvent::Get(get_request) => {
                if let Some(resource_info) = map.get_resource(get_request.id()) {
                    resource_info.check_get_request(&get_request).await;
                }
            }
            RegistryEvent::Exit => {
                self.should_exit = true;
            }
            _ => (),
        }
        Ok(ControlFlow::Continue(()))
    }
}

/// A synchronized reference to a [`ResourceMap`] that allows querying the current state of the
/// parent [`Registry`]
#[derive(Clone)]
#[cfg_attr(feature = "path-any", derive(PathAny))]
pub struct Handle {
    shared: Arc<Shared>,
}

impl crate::input::get::GetRef for ResourceMap {
    type Ref = Handle;
    fn get_ref(&self) -> Self::Ref {
        self.handle()
    }
}

impl Handle {
    /// Returns a set of the [`NamespacedId`]s of each [`Resource`] loaded in the [`Registry`]
    #[must_use]
    pub fn loaded_resources(&self) -> HashSet<NamespacedId> {
        self.shared
            .resources
            .iter()
            .filter(|resource| resource.value().strong_count() != 0)
            .map(|resource| resource.key().clone())
            .collect()
    }

    /// Returns a set of the [`NamespacedId`]s of each [`Bundle`] loaded in the [`Registry`]
    #[must_use]
    pub fn loaded_bundles(&self) -> HashSet<NamespacedId> {
        self.shared
            .bundles
            .iter()
            .filter(|bundle| bundle.value().strong_count() != 0)
            .map(|bundle| bundle.key().clone())
            .collect()
    }

    /// Returns a set of the [`NamespacedId`]s of each [`Resource`] running in the main loop
    #[must_use]
    pub fn running_resources(&self) -> HashSet<NamespacedId> {
        self.shared
            .runners
            .iter()
            .map(|runner| runner.key().clone())
            .collect()
    }
}
