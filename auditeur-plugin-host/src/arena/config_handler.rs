use crate::{arena::State, config::host::PluginGroup, prelude::*};

pub struct ConfigHandler {
    config: config::Host,
}

impl ConfigHandler {
    pub const fn new(config: config::Host) -> Self {
        Self { config }
    }

    pub const fn goal(&self) -> &config::Host {
        &self.config
    }

    pub fn source(&self, id: &SourceId) -> Option<&Source> {
        (self.config.plugins.get(id).map(|group| &group.source))
            .or_else(|| self.config.static_dependencies.get(id))
    }

    pub fn plugin_group(&self, id: &SourceId) -> Option<&PluginGroup> {
        self.config.plugins.get(id)
    }

    pub fn goal_state(&self) -> State {
        State::from_config(&self.config)
    }

    pub fn mutate(&mut self, func: impl FnOnce(&mut config::Host)) {
        func(&mut self.config);
        // TODO: save the new config
    }
}
