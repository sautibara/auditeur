use auditeur::prelude::*;
use auditeur::*;

#[tokio::main]
async fn main() -> Result<(), auditeur::Error> {
    // Here's an example of a typical [`Create`] implementation.
    #[create]
    struct Foo {
        // [`Vec`] doesn't implement [`Input`] or [`Create`], so we'll initialize it manually.
        // Usually, fields without attributes are put above fields with them.
        list: Vec<()>,

        // Since `input` is an [`App`], we could automatically obtain it using [`Input`].
        #[create(input)]
        app: App,

        // [`Bar`] and [`Baz`], on the other hand, implement [`Create`], so we use that instead.
        #[create(create)]
        bar: Bar,
        #[create(create)]
        baz: Baz,
    }

    impl CreateDerive for Foo {
        async fn create((): &Self::External, _: &mut App) -> Result<Self::Plain, Error> {
            Ok(Self::Plain {
                // `list` didn't have any attributes, so we initialize it here.
                list: Vec::new(),

                // `input` is initialized using [`Input`], so we omit it.

                // `bar` and `baz` are initialized using [`Create`],
                // so we give them their external input.
                bar: true,
                baz: (),
            })
        }
    }

    // Custom external input can be specified with `external_input`.
    #[create(external_input(bool))]
    struct Bar {
        boolean: bool,
    }

    impl CreateDerive for Bar {
        // Since we specified the external input, it appears here,
        async fn create(boolean: &bool, _: &mut App) -> Result<Self::Plain, Error> {
            // and we could use it to initialize the type.
            Ok(Self::Plain { boolean: *boolean })
        }
    }

    // If all fields implement [`Default`], we could use `default` to initialize them instead.
    #[create(default)]
    struct Baz {
        number: usize,
    }

    let mut app = App::builder().spawn();

    // Now, when we create [`Foo`],
    let foo = <Foo as Create>::create(&(), &mut app).await?;

    // We could access all the fields however we want.
    assert!(foo.list.is_empty());
    let _ = foo.app.map().get().await;
    assert_eq!(foo.bar.boolean, true);
    assert_eq!(foo.baz.number, 0);

    Ok(())
}
