use auditeur::app::map::{DependencyType, ListenerId, RequestSet};

use auditeur::prelude::*;
use auditeur::*;

#[tokio::main]
async fn main() -> Result<(), auditeur::Error> {
    #[resource("test:resource")]
    #[pure_resource]
    #[create(default)]
    struct Resource;

    let mut app = App::builder().with::<Resource>(()).spawn();

    // App handles' default dependency type is usually [`DependencyType::Unremovable`],
    // but the handle returned by [`AppBuilder::spawn`]'s default is [`DependencyType::Transient`].
    assert_eq!(app.dependent().dependency_type(), DependencyType::Transient);

    // Using [`AppDependentMut`], we could change the dependency type.
    let mut with_dependent = app.dependent_mut().with_ty(DependencyType::Unremovable);

    // Now, if we get a dependency to [`Resource`],
    let dependency = with_dependent
        .components_mut()
        .load(Resource::component_id())
        .await?;
    // its dependency type will be [`DependencyType::Unremovable`].
    assert_eq!(dependency.info().ty, DependencyType::Unremovable);

    // And if we request [`Resource`]'s removal,
    drop(with_dependent);
    app.components_mut()
        .request(RequestSet::remove(Resource::component_id()))
        .await;
    // the request will propagate to the app, since it's `dependency`'s dependent.
    let map = app.map().get().await;
    assert!(
        !map.listeners[&ListenerId::deactivate_component(app.dependent().app_id())]
            .requests
            .is_empty()
    );

    drop(dependency);
    auditeur::test::finish(app, false).await
}
