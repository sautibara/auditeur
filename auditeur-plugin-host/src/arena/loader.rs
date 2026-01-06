use std::{collections::HashMap, path::PathBuf};

use auditeur_plugin::{
    auditeur::app::resource_map::{self, ResourceMapConnection},
    prelude::*,
};

use crate::{PluginName, registry::PluginRegistryConnection, source::Binary};

pub struct LoadedPlugin {
    bundle_id: NamespacedId,
}

#[derive(Default)]
pub struct Loader {
    loaded: HashMap<PluginName, LoadedPlugin>,
}

impl Loader {
    pub async fn load_all(
        &mut self,
        binaries: Vec<Binary>,
        plugin_registry: &PluginRegistryConnection,
    ) -> Result<(), Vec<crate::Error>> {
        let (map, res) = unsafe { plugin_registry.load_binaries(binaries).await };
        self.loaded.extend(
            map.into_iter()
                .map(|(name, bundle_id)| (name, LoadedPlugin { bundle_id })),
        );
        res
    }

    pub async fn unload_all(
        &mut self,
        plugins: Vec<PluginName>,
        resource_map: &ResourceMapConnection,
        registry: &Registry,
    ) -> Result<(), crate::Error> {
        let query: resource_map::Query = plugins
            .iter()
            .filter_map(|plugin| self.loaded.get(plugin))
            .map(|loaded_plugin| &loaded_plugin.bundle_id)
            .cloned()
            .map(resource_map::Query::Bundle)
            .collect();

        let sub_map = resource_map.query(query).await;
        sub_map.unload_in(registry).await;

        for plugin in plugins {
            self.loaded.remove(&plugin);
        }

        Ok(())
    }

    pub fn loaded(&self) -> &HashMap<PluginName, LoadedPlugin> {
        &self.loaded
    }
}
