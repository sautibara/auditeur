use crate::{
    children::HasChildren,
    component::{
        ComponentIdPath, Register, RegistryOf,
        registry::{MarkerLoader, Registries, Registry},
    },
    prelude::*,
};

/// Create a new bundle with the given identifier
///
/// Note: This macro expands to `Bundle::new(ident!($str))`
// TODO: example
#[macro_export]
// the macro is re-exported in the root of the crate, so it needs a specific name
#[allow(clippy::module_name_repetitions)]
macro_rules! new_bundle {
    ($str:tt) => {
        $crate::component::bundle::Bundle::new($crate::namespaced_id::ident!($str))
    };
}

pub struct Bundle {
    id: ComponentIdPath,
    children: Children,
}

impl Bundle {
    pub fn new(id: impl Into<ComponentIdPath>) -> Self {
        Self {
            id: id.into(),
            children: Children::new(),
        }
    }
}

impl Identified for Bundle {
    fn id_dyn(&self) -> &NamespacedIdRef {
        &self.id
    }
}

impl HasChildren for Bundle {
    fn with_erased(
        mut self,
        component: crate::app::registrar::ErasedComponent,
        children: Children,
    ) -> Self {
        self.children = self.children.with_erased(component, children);
        self
    }
}

impl RegistryOf for Bundle {
    type Registry = Bundles;
}

#[identified("auditeur:bundles")]
pub struct Bundles;

impl Registry for Bundles {
    type Loader = MarkerLoader<Self>;
    type Loaded = ();
}

impl RegistryOf for Bundles {
    type Registry = Registries;
}

impl Register<Bundle> for Bundles {
    type ComponentInput = Bundle;

    fn register(
        Bundle { id, children }: Self::ComponentInput,
    ) -> (<Self as Registry>::Loader, Children) {
        (MarkerLoader::new(id), children)
    }
}
