use std::{
    collections::HashMap,
    sync::{Arc, Weak},
};

use anyhow::{anyhow, bail};

use crate::{app::EventSender, prelude::*};

struct LoadedResource {
    id: NamespacedId,
    // hold a strong reference to the parent bundle to keep it alive
    _bundle: Arc<LoadedBundle>,
    // and strong references to all of its dependencies for the same reason
    dependencies: Vec<Arc<LoadedResource>>,
    sender: EventSender,
}

impl Drop for LoadedResource {
    fn drop(&mut self) {
        self.sender.send(RegistryEvent::Unloaded {
            id: self.id.clone(),
        });
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
        self.sender.send(RegistryEvent::BundleUnloaded {
            id: self.id.clone(),
        });
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
#[resource("audituer:resource_map")]
pub struct ResourceMap {
    sender: EventSender,
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
    /// The resources that either aren't loaded or have no references yet
    ///
    /// These can't be put in the main resources map, as they'd need a reference to keep them alive
    resource_queue: HashMap<NamespacedId, LoadedResource>,
    /// The bundles that either aren't loaded or have no references yet
    ///
    /// These can't be put in the main bundles map, as they'd need a reference to keep them alive
    bundle_queue: HashMap<NamespacedId, LoadedBundle>,
}

impl ResourceMap {
    pub(crate) fn new(sender: EventSender) -> Self {
        Self {
            sender,
            runners: HashMap::new(),
            resources: HashMap::new(),
            bundles: HashMap::new(),
            resource_queue: HashMap::new(),
            bundle_queue: HashMap::new(),
        }
    }

    fn add_resource(&mut self, id: NamespacedId, parent: &NamespacedId) -> Result<()> {
        let parent = self.clone_bundle(parent)?;
        self.resource_queue.insert(
            id.clone(),
            LoadedResource {
                id,
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
            self.resources.insert(id.clone(), Arc::downgrade(&resource));
            Ok(resource)
        } else if let Some(resource) = self.resources.get(id).and_then(Weak::upgrade) {
            Ok(resource)
        } else {
            bail!("could not find resource: {id}");
        }
    }

    fn clone_bundle(&mut self, id: &NamespacedId) -> Result<Arc<LoadedBundle>> {
        if let Some(bundle) = self.bundle_queue.remove(id) {
            let bundle = Arc::new(bundle);
            self.bundles.insert(id.clone(), Arc::downgrade(&bundle));
            Ok(bundle)
        } else if let Some(bundle) = self.bundles.get(id).and_then(Weak::upgrade) {
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
}

#[async_trait]
impl<M: Message> Listener<M> for ResourceMap {
    type Input = ();

    async fn handle(&mut self, _: M, (): &mut ()) -> Result<()> {
        Ok(())
    }

    async fn registry_event(&mut self, event: RegistryEvent, (): &mut ()) -> Result<()> {
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
            RegistryEvent::Running { id } => {
                let resource = self.clone_resource(&id)?;
                self.runners.insert(id, resource);
            }
            RegistryEvent::Stopped { id } => {
                self.runners.remove(&id);
            }
            _ => (),
        }
        Ok(())
    }
}
