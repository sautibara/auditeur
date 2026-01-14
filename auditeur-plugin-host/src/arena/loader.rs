use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};

use auditeur_plugin::prelude::*;

use crate::{PluginName, registry::PluginRegistryConnection};

#[derive(Debug)]
#[expect(dead_code)] // may be useful i dunno
pub struct LoadedPlugin {
    bundle_id: NamespacedId,
}

#[derive(Default)]
pub struct Loader {
    loaded: HashMap<PluginName, LoadedPlugin>,
    to_reload: HashSet<PluginName>,
}

impl Loader {
    pub async fn load_all(
        &mut self,
        binaries: Vec<(PluginName, PathBuf)>,
        plugin_registry: &PluginRegistryConnection,
    ) -> Result<(), Vec<crate::Error>> {
        let (map, res) = unsafe { plugin_registry.load_binaries(binaries).await };
        self.to_reload.retain(|name| !map.contains_key(name));
        self.loaded.extend(
            map.into_iter()
                .map(|(name, bundle_id)| (name, LoadedPlugin { bundle_id })),
        );
        res
    }

    // pub async fn unload_all(
    //     &mut self,
    //     plugins: Vec<PluginName>,
    //     resource_map: &ResourceMapConnection,
    //     registry: &Registry,
    // ) -> Result<(), crate::Error> {
    //     let query: resource_map::Query = plugins
    //         .iter()
    //         .filter_map(|plugin| self.loaded.get(plugin))
    //         .map(|loaded_plugin| &loaded_plugin.bundle_id)
    //         .cloned()
    //         .map(resource_map::Query::Bundle)
    //         .collect();
    //
    //     let sub_map = resource_map.query(query).await;
    //     sub_map.unload_in(registry).await;
    //
    //     for plugin in plugins {
    //         self.loaded.remove(&plugin);
    //         self.to_reload.remove(&plugin);
    //     }
    //
    //     Ok(())
    // }

    pub const fn loaded(&self) -> &HashMap<PluginName, LoadedPlugin> {
        &self.loaded
    }

    pub fn add_loaded(&mut self, name: PluginName, bundle_id: NamespacedId) {
        self.loaded.insert(name, LoadedPlugin { bundle_id });
    }

    pub fn queue_reload(&mut self, name: PluginName) {
        if self.loaded.contains_key(&name) {
            self.to_reload.insert(name);
        }
    }

    pub fn to_reload(&self, name: &PluginName) -> bool {
        self.to_reload.contains(name)
    }
}
