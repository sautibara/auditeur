use std::{collections::HashSet, path::PathBuf};

use auditeur_plugin::{
    Plugins,
    auditeur::{
        app::{
            AppFn, AppFnOnce, AppMutFnOnce,
            event::Event,
            map::{ComponentStateChange, ListenerId, RequestAction, RequestSet},
        },
        component::ComponentIdPath,
    },
    prelude::*,
};

use crate::PluginName;

#[derive(Default)]
pub struct Loader {
    loaded: HashSet<PluginName>,
    to_reload: HashSet<PluginName>,
}

impl Loader {
    fn plugin_component_ids(
        iter: impl IntoIterator<Item = PluginName>,
    ) -> impl Iterator<Item = ComponentId> {
        iter.into_iter().map(|plugin| ComponentId {
            registry: Plugins::id(),
            component: ComponentIdPath::Shared(plugin.0),
        })
    }

    fn remove_plugins(iter: impl IntoIterator<Item = PluginName>) -> RequestSet {
        Self::plugin_component_ids(iter)
            .map(RequestAction::RemoveComponent)
            .collect::<RequestSet>()
    }

    pub async fn load_all(
        &mut self,
        binaries: Vec<(PluginName, PathBuf)>,
        app: &mut App,
    ) -> Result<(), Vec<crate::Error>> {
        let mut errors: Vec<crate::Error> = Vec::new();

        let plugin_names = || binaries.iter().map(|(plugin, _)| plugin.clone());

        // Request the unloading of any plugins that are already loaded.
        let request = Self::remove_plugins(plugin_names());
        let ((map, events), waiter) = (AppFn.map().get().using_mut())
            .then(AppFn.get_event_listener())
            .then(AppFn.components_mut().request(request))
            .call(app)
            .await;

        // Figure out what components we unloaded that we don't want unloaded.
        // That is, components that depend on any of the plugins we're reloading.
        let mut to_load = HashSet::new();
        if let Ok(mut events) = events.map_err(|err| errors.push(err.into())) {
            let ids = Self::plugin_component_ids(plugin_names());
            let descendants = map.descendants_of(ids);

            let our_request = waiter.request().clone();

            while let Some(event) = events.listen().await {
                match event {
                    Event::RequestSuccess { request } if request == our_request => {
                        break;
                    }
                    Event::Requested {
                        listener:
                            ListenerId::Component {
                                id,
                                change: ComponentStateChange::Unload(_),
                            },
                        request,
                    } if request == our_request && !descendants.contains(&id) => {
                        to_load.insert(id);
                    }
                    _ => (),
                }
            }
        } else {
            waiter.wait().await;
        }

        // Now that the old stuff is unloaded, load the new stuff.
        let mut loaded = HashSet::new();
        let children = binaries
            .iter()
            .fold(Children::new(), |children, (id, path)| {
                let plugin = unsafe {
                    auditeur_plugin::PluginComponent::load_from_file_unchecked(path, id.0.clone())
                };

                let Ok(plugin) = plugin.map_err(|err| errors.push(err.into())) else {
                    return children;
                };

                loaded.insert(id.clone());
                children.with::<auditeur_plugin::PluginComponent>(plugin)
            });
        app.components_mut().add(children).await;

        // And reload the stuff we didn't want to unload.
        for id in to_load {
            let _ = app.components_mut().load_lazy(id.into_id()).await;
        }

        self.to_reload.retain(|name| !loaded.contains(name));
        self.loaded.extend(loaded);

        errors.is_empty().then_some(()).ok_or(errors)
    }

    pub async fn unload_all(&mut self, plugins: Vec<PluginName>, app: &mut App) {
        self.loaded.retain(|k| !plugins.contains(k));
        self.to_reload.retain(|k| !plugins.contains(k));

        let request = Self::remove_plugins(plugins);
        let waiter = app.components_mut().request(request).await;
        waiter.wait().await;
    }

    pub const fn loaded(&self) -> &HashSet<PluginName> {
        &self.loaded
    }

    pub fn queue_reload(&mut self, name: PluginName) {
        if self.loaded.contains(&name) {
            self.to_reload.insert(name);
        }
    }

    pub fn to_reload(&self, name: &PluginName) -> bool {
        self.to_reload.contains(name)
    }
}
