//! A dependency tree of the [resources](mod@crate::resource) and [bundles](crate::Bundle) that are
//! currently loaded

use std::{
    collections::{HashMap, HashSet},
    fmt::Debug,
    sync::{Arc, Weak},
};

use anyhow::{anyhow, bail};

use crate::{app::EventSender, prelude::*, resource::listener};

use super::EventReceiver;
#[cfg(feature = "runtime-loading")]
use super::GetRequest;

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

#[allow(clippy::used_underscore_binding)]
impl Debug for LoadedResource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut debug_struct = f.debug_struct("LoadedResource");
        debug_struct.field("id", &self.id);
        #[cfg(feature = "runtime-loading")]
        debug_struct.field(
            "resource",
            &if self.resource.blocking_read().is_some() {
                "<present>"
            } else {
                "<empty>"
            },
        );
        debug_struct
            .field("_bundle", &self._bundle)
            .field("dependencies", &self.dependencies)
            .field("sender", &self.sender)
            .finish()
    }
}

impl Drop for LoadedResource {
    fn drop(&mut self) {
        self.sender.send(RegistryEvent::Unloaded(self.id.clone()));
    }
}

#[cfg(feature = "runtime-loading")]
impl LoadedResource {
    async fn supply_owned(&self, resource: Box<dyn super::LoadedResource>) {
        tracing::trace!(
            "adding {} to the ResourceMap as a NonRunnable",
            resource.ident()
        );
        let mut resource_option = self.resource.write().await;
        *resource_option = Some(resource);
    }

    /// Runs `func` on the contained owned resource, if it exists
    async fn check_get_request(&self, get_request: &GetRequest, registry: &Registry) {
        let resource_option = self.resource.read().await;
        if let Some(resource) = resource_option.as_deref() {
            get_request.check_ref(resource.resource(), registry).await;
        }
    }
}

#[derive(Debug)]
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
/// This listener is automatically loaded to every [`App`] that is constructed
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
    respond,
    components((component = MainComponent, input = ())),
)]
pub struct ResourceMap {
    sender: EventSender,
    /// The resources that either aren't loaded or have no references yet
    ///
    /// These can't be put in the main resources map, as they'd need a reference to keep them alive
    resource_queue: HashMap<NamespacedId, LoadedResource>,
    /// The bundles that either aren't loaded or have no references yet
    ///
    /// These can't be put in the main bundles map, as they'd need a reference to keep them alive
    bundle_queue: HashMap<NamespacedId, LoadedBundle>,
    /// A map of all of the currently running resources
    ///
    /// These resources are the roots of what stays loaded, and as such, are stored as strong
    /// references
    runners: HashMap<NamespacedId, Arc<LoadedResource>>,
    /// A map of all of the resources that have ever been loaded
    ///
    /// If the underlying Arc unloads (no strong references point to it anymore), that means that
    /// the resource also must be unloaded. As such, this map holds no strong references to any of
    /// these resources, allowing them to unload if necessary.
    resources: HashMap<NamespacedId, Weak<LoadedResource>>,
    /// A map of all of the bundles that have ever been loaded
    bundles: HashMap<NamespacedId, Weak<LoadedBundle>>,
}

#[create]
impl Create for ResourceMap {
    async fn create(sender: EventSender) -> Result<Self> {
        Ok(Self {
            sender,
            resource_queue: HashMap::new(),
            bundle_queue: HashMap::new(),
            runners: HashMap::new(),
            resources: HashMap::new(),
            bundles: HashMap::new(),
        })
    }
}

impl ResourceMap {
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
            self.resources.insert(id, Arc::downgrade(&resource));
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
            self.bundles.insert(id, Arc::downgrade(&bundle));
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
        self.resources.get(id).and_then(Weak::upgrade)
    }

    fn get_bundle(&self, id: &NamespacedIdRef) -> Option<Arc<LoadedBundle>> {
        self.bundles.get(id).and_then(Weak::upgrade)
    }
}

#[doc(hidden)]
pub struct MainComponent {
    receiver: EventReceiver,
    #[cfg(feature = "runtime-loading")]
    registry: Registry,
    should_exit: bool,
}

impl Input for MainComponent {
    async fn create(registry: &Registry, _: &NamespacedIdRef) -> Result<Self, Error> {
        Ok(Self {
            receiver: registry.event_receiver(),
            #[cfg(feature = "runtime-loading")]
            registry: registry.handle(),
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
                map.runners.insert(id, resource);
            }
            RegistryEvent::Stopped(id) => {
                map.runners.remove(&id);
                // this should be the last runner
                if self.should_exit && map.runners.len() <= 1 {
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
                    resource_info
                        .check_get_request(&get_request, &self.registry)
                        .await;
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

#[respond("auditeur:resource_map", get_ref)]
impl ResourceMap {
    /// Returns a set of the [`NamespacedId`]s of each [`Resource`] loaded in the [`Registry`]
    #[must_use]
    pub fn loaded_resources(&self, (): ()) -> HashSet<NamespacedId> {
        self.resources
            .iter()
            .filter(|(_, resource)| resource.strong_count() != 0)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Returns a set of the [`NamespacedId`]s of each [`Bundle`] loaded in the [`Registry`]
    #[must_use]
    pub fn loaded_bundles(&self, (): ()) -> HashSet<NamespacedId> {
        self.bundles
            .iter()
            .filter(|(_, bundle)| bundle.strong_count() != 0)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Returns a set of the [`NamespacedId`]s of each [`Resource`] running in the main loop
    #[must_use]
    pub fn running_resources(&self, (): ()) -> HashSet<NamespacedId> {
        self.runners.keys().cloned().collect()
    }
}
