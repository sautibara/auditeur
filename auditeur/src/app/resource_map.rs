//! A dependency tree of the [resources](mod@crate::resource) and [bundles](crate::Bundle) that are
//! currently loaded

use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, RwLock, Weak},
};

use anyhow::{anyhow, bail};
use dashmap::DashMap;
use tracing::trace;

use crate::{app::EventSender, prelude::*};

use super::EventReceiver;

struct LoadedResource {
    id: NamespacedId,
    #[cfg(feature = "runtime-loading")]
    resource: RwLock<Option<Box<dyn super::LoadedResource>>>,
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
    fn supply_owned(&self, resource: Box<dyn super::LoadedResource>) {
        trace!(
            "adding {} to the ResourceMap as a NonRunnable",
            resource.ident()
        );
        let mut resource_option = self
            .resource
            .write()
            .expect("ResourceMap LoadedResource RwLock shouldn't be poisoned");
        *resource_option = Some(resource);
    }

    /// Runs `func` on the contained owned resource, if it exists
    fn with_owned(&self, func: impl FnOnce(&dyn super::LoadedResource)) {
        let resource_option = self
            .resource
            .read()
            .expect("ResourceMap LoadedResource RwLock shouldn't be poisoned");
        if let Some(resource) = resource_option.as_deref() {
            func(resource);
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
#[resource("auditeur:resource_map")]
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
impl<M: Message> Create<M> for ResourceMap {
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

    fn add_resource(&mut self, id: NamespacedId, parent: &NamespacedId) -> Result<()> {
        let parent = self.clone_bundle(parent)?;
        self.resource_queue.insert(
            id.clone(),
            LoadedResource {
                id,
                #[cfg(feature = "runtime-loading")]
                resource: RwLock::new(None),
                _bundle: parent,
                dependencies: Vec::new(),
                sender: self.sender.clone(),
            },
        );
        Ok(())
    }

    fn add_bundle(&mut self, id: NamespacedId, parent: Option<&NamespacedId>) -> Result<()> {
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

    fn clone_resource(&mut self, id: &NamespacedId) -> Result<Arc<LoadedResource>> {
        if let Some(resource) = self.resource_queue.remove(id) {
            let resource = Arc::new(resource);
            self.shared
                .resources
                .insert(id.clone(), Arc::downgrade(&resource));
            Ok(resource)
        } else if let Some(resource) = self.get_resource(id) {
            Ok(resource)
        } else {
            bail!("could not find resource: {id}");
        }
    }

    fn clone_bundle(&mut self, id: &NamespacedId) -> Result<Arc<LoadedBundle>> {
        if let Some(bundle) = self.bundle_queue.remove(id) {
            let bundle = Arc::new(bundle);
            self.shared
                .bundles
                .insert(id.clone(), Arc::downgrade(&bundle));
            Ok(bundle)
        } else if let Some(bundle) = self.get_bundle(id) {
            Ok(bundle)
        } else {
            bail!("could not find bundle: {id}");
        }
    }

    fn add_reference(&mut self, by: &NamespacedId, to: &NamespacedId) -> Result<()> {
        let to = self.clone_resource(to)?;
        let by = self
            .resource_queue
            .get_mut(by)
            .ok_or_else(|| anyhow!("could not find resource: {by}"))?;
        by.dependencies.push(to);
        Ok(())
    }

    fn get_resource(&self, id: &NamespacedId) -> Option<Arc<LoadedResource>> {
        self.shared
            .resources
            .get(id)
            .and_then(|resource| resource.upgrade())
    }

    fn get_bundle(&self, id: &NamespacedId) -> Option<Arc<LoadedBundle>> {
        self.shared
            .bundles
            .get(id)
            .and_then(|resource| resource.upgrade())
    }
}

#[async_trait]
impl<M: Message> Watcher<M> for ResourceMap {
    type Input = (EventReceiver, Stopper);

    // insta-stop trick to control stopping
    // this should be one of the last resources to exit
    async fn run(&mut self, (_, stopper): &mut Self::Input) -> Result<()> {
        stopper.stop::<Self>();
        Ok(())
    }

    async fn exit(&mut self, (receiver, _): &mut Self::Input) -> Result<()> {
        let mut should_exit = false;
        while let Some(event) = receiver.recv().await {
            match event {
                RegistryEvent::BundleLoading { id, parent, .. } => {
                    self.add_bundle(id, parent.as_ref())?;
                }
                RegistryEvent::Loading { id, bundle } => {
                    self.add_resource(id, &bundle)?;
                }
                RegistryEvent::Reference { by, to } => {
                    self.add_reference(&by, &to)?;
                }
                RegistryEvent::Running(id) => {
                    let resource = self.clone_resource(&id)?;
                    self.shared.runners.insert(id, resource);
                }
                RegistryEvent::Stopped(id) => {
                    self.shared.runners.remove(&id);
                    // this should be the last runner
                    if should_exit && self.shared.runners.len() <= 1 {
                        break;
                    }
                }
                #[cfg(feature = "runtime-loading")]
                RegistryEvent::NonRunnable(supplier) => {
                    if let Some(resource) = self.get_resource(&supplier.id().into_owned()) {
                        if let Some(owned) = supplier.take() {
                            resource.supply_owned(owned);
                        }
                    }
                }
                #[cfg(feature = "runtime-loading")]
                RegistryEvent::Get(get_request) => {
                    if let Some(resource_info) = self.get_resource(&get_request.id().into_owned()) {
                        resource_info.with_owned(|owned| get_request.check_ref(owned.resource()));
                    }
                }
                RegistryEvent::Exit => {
                    should_exit = true;
                }
                _ => (),
            }
        }
        Ok(())
    }
}

/// A synchronized reference to a [`ResourceMap`] that allows querying the current state of the
/// parent [`Registry`]
#[derive(Clone)]
#[cfg_attr(feature = "path-any", derive(PathAny))]
pub struct Handle {
    shared: Arc<Shared>,
}

impl crate::input::GetRef for ResourceMap {
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
