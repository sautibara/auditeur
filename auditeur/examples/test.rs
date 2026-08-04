use auditeur::app::map::RequestSet;
use auditeur::component::bundle::Bundles;

use auditeur::prelude::*;
use auditeur::*;

#[tokio::main]
async fn main() -> Result<(), Error> {
    // The loading and unloading of bundles is useless, as their loaded value is empty,
    // but they can still be used to investigate how loading and unloading work.

    let mut app = App::builder()
        .with::<Bundle>(new_bundle!("test:bundle"))
        .spawn();

    let bundle_id = ComponentId::inn::<Bundles>(ident!("test:bundle"));

    // All components start out unloaded and only get loaded when necessary. (Although, some
    // registries immediately load their components when they're added, like resources).
    let map = app.map().get().await;
    assert!(map.is_added(&bundle_id));
    assert!(!map.is_loaded(&bundle_id));

    // By using [`AppComponentsMut::load`], we can ensure that a component is loaded.
    let dependency: Dependency<()> = app.components_mut().load(bundle_id.clone()).await?;

    // [`AppComponentsMut::load`] only returns once the component finishes unloading,
    // so we can be sure that it's loaded now.
    let map = app.map().get().await;
    assert!(map.is_loaded(&bundle_id));

    // Dependencies keep components loaded and provide access to what they're wrapping.
    // They also hold extra information that can be accessed through [`Dependency::info_of`].
    let info = Dependency::info_of(&dependency);

    // For example, we could check who the dependent of the dependency is - who is keeping it alive.
    let dependent = info.id.dependent();
    // By default, the dependent of the app handle given through [`app::Builder::spawn`] is the app
    // component, so that should also be the dependent of our dependency.
    assert_eq!(**dependent, App::component_id());

    // Once all dependencies of a component are dropped, it is automatically unloaded.
    drop(dependency);
    let waiter = app
        .components_mut()
        .wait_for(RequestSet::unload(bundle_id.clone()))
        .await;
    waiter.wait().await;

    let map = app.map().get().await;
    assert!(!map.is_loaded(&bundle_id));

    auditeur::test::finish(app, false).await
}
