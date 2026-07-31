use crate::{
    app::registrar::ErasedComponent,
    component::{Register, RegistryOf},
    prelude::*,
};

/// A collection of [`Component`]s.
#[derive(Default, Debug)]
pub struct Children {
    current_parent: Option<ComponentId>,
    children: Vec<Child>,
}

#[derive(Debug)]
pub(crate) struct Child {
    pub(crate) parent: Option<ComponentId>,
    pub(crate) component: ErasedComponent,
}

impl Children {
    /// Creates a new, empty collection.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn combine(mut self, mut children: Self) -> Self {
        if let Some(current_parent) = self.current_parent.clone() {
            children.adopt_by(current_parent);
        }
        self.children.extend(children.into_children());
        self
    }

    pub(crate) fn adopt_by(&mut self, new_parent: ComponentId) {
        for child in &mut self.children {
            child.parent.get_or_insert_with(|| new_parent.clone());
        }
        self.current_parent = Some(new_parent);
    }

    pub(crate) fn adopted_by(mut self, new_parent: ComponentId) -> Self {
        self.adopt_by(new_parent);
        self
    }

    fn with_parent(mut self, parent: ComponentId, func: impl FnOnce(Self) -> Self) -> Self {
        let old_parent = self.current_parent.replace(parent);
        self = func(self);
        self.current_parent = old_parent;
        self
    }

    pub(crate) fn into_children(self) -> impl Iterator<Item = Child> {
        self.children.into_iter()
    }
}

impl HasChildren for Children {
    fn with_erased(mut self, component: ErasedComponent, children: Children) -> Self {
        let id = component.object.component_id_dyn();

        // Add the component first and its children after.
        self.children.push(Child {
            parent: self.current_parent.clone(),
            component,
        });
        self = self.with_parent(id, |this| this.combine(children));

        self
    }
}

pub trait HasChildren: Sized {
    #[must_use]
    fn with_erased(self, component: ErasedComponent, children: Children) -> Self;

    /// Adds a component with a specified registry (see [`RegistryOf`]) to the collection.
    ///
    /// This is identical to calling [`Self::within`] with the registry [`C::Registry`].
    #[must_use]
    fn with<C: RegistryOf>(self, input: <C::Registry as Register<C>>::ComponentInput) -> Self {
        self.within::<C::Registry, C>(input)
    }

    /// Adds a component to this collection under a specific registry.
    ///
    /// All children of this component are also immediately added to the collection.
    #[must_use]
    fn within<R: Register<C>, C>(self, input: <R as Register<C>>::ComponentInput) -> Self {
        let (component, children) = ErasedComponent::new::<R, C>(input);
        self.with_erased(component, children)
    }
}
