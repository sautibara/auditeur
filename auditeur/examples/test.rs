use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use auditeur::component::resource::get::{GetCreate, GetRefWith};
use auditeur::prelude::*;
use auditeur::*;

#[tokio::main]
async fn main() -> Result<(), Error> {
    #[resource("test:resource")]
    #[pure_resource]
    #[create(default)]
    struct Resource {
        number: Arc<AtomicUsize>,
    }

    #[create(external_input(Resource), derive_for_plain(Clone))]
    #[derive(auditeur::Any)]
    struct Ref {
        // The handle can have both values from the original resource,
        number: Arc<AtomicUsize>,

        // and extra values obtained through the app.
        #[create(input)]
        app: App,
    }

    impl CreateDerive for Ref {
        async fn create(resource: &Resource, _: &mut App) -> Result<Self::Plain, Error> {
            Ok(Self::Plain {
                // To obtain values from the resource, use the external input.
                number: resource.number.clone(),
            })
        }
    }

    impl GetRefWith for Resource {
        type Method = GetCreate<Ref>;
    }

    let mut app = App::builder().with::<Resource>(()).spawn();

    let resource = app.resources_mut().get::<Resource>().await?;

    // Now we can use both the values from the original resource,
    assert_eq!(resource.number.load(Ordering::Relaxed), 0);

    // and the values obtained through the app.
    let map = resource.app.map().get().await;
    assert!(map.is_added(&Resource::component_id()));

    drop(resource);
    auditeur::test::finish(app, false).await
}
