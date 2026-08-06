macro_rules! app_future_def {
    // Mutable components.
    ( @ true (@($when_false:tt, $when_true:tt) $($in:tt)*) ($($out:tt)*) $($rest:tt)*) => {
        app_future_def!( @ true ($($in)*) ( $($out)* $when_true ) $($rest)* );
    };
    ( @ false (@($when_false:tt, $when_true:tt) $($in:tt)*) ($($out:tt)*) $($rest:tt)*) => {
        app_future_def!( @ false ($($in)*) ( $($out)* $when_false ) $($rest)* );
    };
    ( @ true (@mut $($in:tt)*) ($($out:tt)*) $($rest:tt)*) => {
        app_future_def!( @ true ($($in)*) ( $($out)* mut ) $($rest)* );
    };
    ( @ false (@mut $($in:tt)*) ($($out:tt)*) $($rest:tt)*) => {
        app_future_def!( @ false ($($in)*) ( $($out)* ) $($rest)* );
    };
    ( @ true (@AppFnOnce $($in:tt)*) ($($out:tt)*) $($rest:tt)*) => {
        app_future_def!( @ true ($($in)*) ( $($out)* AppMutFnOnce ) $($rest)* );
    };
    ( @ false (@AppFnOnce $($in:tt)*) ($($out:tt)*) $($rest:tt)*) => {
        app_future_def!( @ false ($($in)*) ( $($out)* AppFnOnce ) $($rest)* );
    };
    ( @ true (@AppFuture $($in:tt)*) ($($out:tt)*) $($rest:tt)*) => {
        app_future_def!( @ true ($($in)*) ( $($out)* AppMutFuture ) $($rest)* );
    };
    ( @ false (@AppFuture $($in:tt)*) ($($out:tt)*) $($rest:tt)*) => {
        app_future_def!( @ false ($($in)*) ( $($out)* AppFuture ) $($rest)* );
    };
    ( @ true (@AppFnOnceInner $($in:tt)*) ($($out:tt)*) $($rest:tt)*) => {
        app_future_def!( @ true ($($in)*) ( $($out)* AppMutFnOnceInner ) $($rest)* );
    };
    ( @ false (@AppFnOnceInner $($in:tt)*) ($($out:tt)*) $($rest:tt)*) => {
        app_future_def!( @ false ($($in)*) ( $($out)* AppFnOnceInner ) $($rest)* );
    };
    ( @ true (@AppFnOnceImpl $($in:tt)*) ($($out:tt)*) $($rest:tt)*) => {
        app_future_def!( @ true ($($in)*) ( $($out)* AppMutFnOnceImpl ) $($rest)* );
    };
    ( @ false (@AppFnOnceImpl $($in:tt)*) ($($out:tt)*) $($rest:tt)*) => {
        app_future_def!( @ false ($($in)*) ( $($out)* AppFnOnceImpl ) $($rest)* );
    };
    ( @ true (@AppFutureImpl $($in:tt)*) ($($out:tt)*) $($rest:tt)*) => {
        app_future_def!( @ true ($($in)*) ( $($out)* AppMutFutureImpl ) $($rest)* );
    };
    ( @ false (@AppFutureImpl $($in:tt)*) ($($out:tt)*) $($rest:tt)*) => {
        app_future_def!( @ false ($($in)*) ( $($out)* AppFutureImpl ) $($rest)* );
    };
    // Big components.
    ( @ $mut:tt (@fields $($in:tt)*) ($($out:tt)*) $($rest:tt)*) => {
        app_future_def!( @ $mut ($($in)*) ( $($out)*
            source: Source,
            handle_fn: HandleFn,
            owned_fn: OwnedFn
        ) $($rest)* );
    };
    ( @ $mut:tt (@generics_bounded $($in:tt)*) ($($out:tt)*) $($rest:tt)*) => {
        app_future_def!( @ $mut (
            Source: @AppFnOnce<Output = Output>,
            Output: Send + 'static,
            HandleFn: FnOnce(Source, &'a HandleData) -> (Method, HandleFut)
                + Send + 'static,
            HandleFut: Future<Output = Output> + Send + 'a,
            OwnedFn: FnOnce(Source, &'a @mut Owned, &'a HandleData) -> Output
                + Send + 'static,
        $($in)*) ( $($out)* ) $($rest)*);
    };
    ( @ $mut:tt (@generics_unbounded $($in:tt)*) ($($out:tt)*) $($rest:tt)*) => {
        app_future_def!( @ $mut ($($in)*) ( $($out)*
            Source,
            Output,
            HandleFn,
            HandleFut,
            OwnedFn,
        ) $($rest)* );
    };
    ( @ $mut:tt (@type_defs $($in:tt)*) ($($out:tt)*) $($rest:tt)*) => {
        app_future_def!( @ $mut (
            type Source: @AppFnOnce<Output = Self::Output>;
            type Output: Send + 'static;
            type HandleFn: FnOnce(Self::Source, &'a HandleData) -> (Method, Self::HandleFut)
                + Send + 'static;
            type HandleFut: Future<Output = Self::Output> + Send + 'a;
            type OwnedFn: FnOnce(Self::Source, &'a @mut Owned, &'a HandleData)
                -> Self::Output
                + Send + 'static
        $($in)*) ($($out)*) $($rest)*);
    };
    ( @ $mut:tt (@type_impls $($in:tt)*) ($($out:tt)*) $($rest:tt)*) => {
        app_future_def!( @ $mut ($($in)*) ( $($out)*
            type Source = Source;
            type Output = Output;
            type HandleFn = HandleFn;
            type HandleFut = HandleFut;
            type OwnedFn = OwnedFn
        ) $($rest)*);
    };
    ( @ $mut:tt (@generics_from_types $($in:tt)*) ($($out:tt)*) $($rest:tt)*) => {
        app_future_def!( @ $mut ($($in)*) ( $($out)*
            Self::Source,
            Self::Output,
            Self::HandleFn,
            Self::HandleFut,
            Self::OwnedFn
        ) $($rest)*);
    };
    // Pushing a delimiter onto the delimiter stack.
    ( @ $mut:tt (($($next:tt)*) $($in:tt)*) ($($out:tt)*) $($rest:tt)*) => {
        app_future_def!( @ $mut ($($next)*) () paren ($($in)*) ($($out)*) $($rest)*);
    };
    ( @ $mut:tt ({$($next:tt)*} $($in:tt)*) ($($out:tt)*) $($rest:tt)*) => {
        app_future_def!( @ $mut ($($next)*) () curly ($($in)*) ($($out)*) $($rest)*);
    };
    ( @ $mut:tt ([$($next:tt)*] $($in:tt)*) ($($out:tt)*) $($rest:tt)*) => {
        app_future_def!( @ $mut ($($next)*) () square ($($in)*) ($($out)*) $($rest)*);
    };
    // Pushing a default token.
    ( @ $mut:tt ($next:tt $($in:tt)*) ($($out:tt)*) $($rest:tt)* ) => {
        app_future_def!( @ $mut ($($in)*) ($($out)* $next) $($rest)* );
    };
    // Popping a delimiter off the delimiter stack.
    ( @ $mut:tt () ($($out:tt)*) paren ($($in_next:tt)*) ($($out_next:tt)*) $($rest:tt)*) => {
        app_future_def!( @ $mut ($($in_next)*) ($($out_next)* ($($out)*)) $($rest)* );
    };
    ( @ $mut:tt () ($($out:tt)*) curly ($($in_next:tt)*) ($($out_next:tt)*) $($rest:tt)*) => {
        app_future_def!( @ $mut ($($in_next)*) ($($out_next)* {$($out)*}) $($rest)* );
    };
    ( @ $mut:tt () ($($out:tt)*) square ($($in_next:tt)*) ($($out_next:tt)*) $($rest:tt)*) => {
        app_future_def!( @ $mut ($($in_next)*) ($($out_next)* [$($out)*]) $($rest)* );
    };
    // Base case.
    ( @ $mut:tt () ($($out:tt)*) ) => {
        $($out)*
    };
    // Initial entrance.
    ( mut: $mut:tt; $($in:tt)* ) => {
        app_future_def!( @ $mut ($($in)*) () );
    };
    ( $($in:tt)* ) => {
        app_future_def!( @ false ($($in)*) () );
        app_future_def!( @ true ($($in)*) () );
    };
}

use std::fmt::Debug;
use std::marker::PhantomData;

use tokio::sync::oneshot;

use crate::{
    app::{AppFn, Event, HandleData, Inner, Method, Owned},
    component::listen::Listener,
    prelude::*,
};

/// A [`Future`] extension returned by an immutable method on [`App`].
///
/// See the [module documentation](super#app-functions--futures) for more information.
#[must_use]
pub trait AppFuture<'a>: IntoFuture<IntoFuture: Send, Output: Send> + Send + Sized {
    /// Converts this future back into the [`AppFnOnce`] that it was created with, alongside
    /// the app reference that was used.
    fn into_parts(self) -> (&'a App, impl AppFnOnce<Output = Self::Output>);

    /// Converts this future back into the [`AppFnOnce`] that it was created with, discarding
    /// the reference.
    fn into_fn_once(self) -> impl AppFnOnce<Output = Self::Output> {
        let (_, fn_once) = self.into_parts();
        fn_once
    }
}

/// A [`Future`] extension returned by a mutable method on [`App`].
///
/// See the [module documentation](super#app-functions--futures) for more information.
#[must_use]
pub trait AppMutFuture<'a>: IntoFuture<IntoFuture: Send, Output: Send> + Send + Sized {
    /// Converts this future back into the [`AppMutFnOnce`] that it was created with, alongside
    /// the app reference that was used.
    fn into_parts(self) -> (&'a mut App, impl AppMutFnOnce<Output = Self::Output>);

    /// Converts this future back into the [`AppMutFnOnce`] that it was created with, discarding
    /// the reference.
    fn into_fn_once(self) -> impl AppMutFnOnce<Output = Self::Output> {
        let (_, fn_once) = self.into_parts();
        fn_once
    }

    /// Converts this future into one that also obtains an [event listener](App::get_event_listener)
    /// from the [`App`].
    ///
    /// This is not supported for [`AppFuture`], since getting an event listener requires a mutable
    /// reference. Instead, do the same thing using [`AppMutFnOnce`]s.
    fn with_event_listener(
        self,
    ) -> impl AppMutFuture<'a, Output = (Self::Output, Result<Listener<Event>, Error>)> {
        let (app, fn_once) = self.into_parts();
        let fn_once = fn_once.then(AppFn.get_event_listener());
        fn_once.call(app)
    }
}

app_future_def! {
    struct @AppFutureImpl<'a, F: @AppFnOnceInner<'a>> {
        app: &'a @mut App,
        func: F,
    }

    impl<'a, F: @AppFnOnceInner<'a>> IntoFuture for @AppFutureImpl<'a, F> {
        type Output = F::Output;
        type IntoFuture = futures::future::Either<F::HandleFut, futures::future::Ready<F::Output>>;

        fn into_future(self) -> Self::IntoFuture {
            let func: @AppFnOnceImpl<_, _, _, _, _> = self.func.into();
            match & @mut self.app.variant {
                Inner::Handle => {
                    let (method, future) = (func.handle_fn)(func.source, &self.app.handle);
                    let _ = self.app.handle.method_sender.send(method);
                    futures::future::Either::Left(future)
                }
                Inner::Owned(owned) => {
                    futures::future::Either::Right(futures::future::ready(
                        (func.owned_fn)(func.source, owned, &self.app.handle)
                    ))
                }
            }
        }
    }

    impl<'a, F: @AppFnOnceInner<'a>> @AppFuture<'a> for @AppFutureImpl<'a, F> {
        fn into_parts(self) -> (&'a @mut App, impl @AppFnOnce<Output = Self::Output>) {
            (self.app, self.func.into().source)
        }
    }
}

/// An immutable method for an [`App`] that has not been given a handle yet.
///
/// See the [module documentation](super#app-functions--futures) for more information.
#[must_use]
pub trait AppFnOnce: Send + Sized + 'static {
    type Output: Send + 'static;

    #[doc(hidden)]
    fn into_inner<'a>(self) -> impl AppFnOnceInner<'a, Output = Self::Output>;

    /// Call this method by providing an app handle.
    fn call(self, app: &App) -> impl AppFuture<'_, Output = Self::Output> {
        AppFutureImpl {
            app,
            func: self.into_inner(),
        }
    }

    /// Create an [`AppMutFnOnce`] that calls this method.
    ///
    /// Think of this as calling an immutable method with a mutable reference.
    fn using_mut(self) -> impl AppMutFnOnce<Output = Self::Output> {
        struct UsingMut<A: AppFnOnce>(A);

        impl<A: AppFnOnce> AppMutFnOnce for UsingMut<A> {
            type Output = A::Output;
            fn into_inner<'a>(self) -> impl AppMutFnOnceInner<'a, Output = Self::Output> {
                AppMutFnOnceImpl {
                    pd: PhantomData,
                    source: self,
                    handle_fn: |this, handle| {
                        let this: AppFnOnceImpl<_, _, _, _, _> = this.0.into_inner().into();
                        (this.handle_fn)(this.source, handle)
                    },
                    owned_fn: move |this, owned, handle| {
                        let this: AppFnOnceImpl<_, _, _, _, _> = this.0.into_inner().into();
                        (this.owned_fn)(this.source, owned, handle)
                    },
                }
            }
        }

        UsingMut::<Self>(self)
    }

    /// Creates a new [`AppFnOnce`] that performs this action first, performs the `other` action
    /// right after, and wraps their outputs in a tuple.
    fn then<U: Send + 'static>(
        self,
        other: impl AppFnOnce<Output = U>,
    ) -> impl AppFnOnce<Output = (Self::Output, U)> {
        Then {
            before: self,
            after: other,
        }
    }

    /// Creates a new [`AppFnOnce`] that performs this action and maps its output using the
    /// `mapping`.
    fn map<U: Send + 'static>(
        self,
        mapping: impl FnOnce(Self::Output) -> U + Send + 'static,
    ) -> impl AppFnOnce<Output = U> {
        AppFnMapping {
            inner: self,
            mapping,
        }
    }
}

/// A mutable method for an [`App`] that has not been given a handle yet.
///
/// See the [module documentation](super#app-functions--futures) for more information.
#[must_use]
pub trait AppMutFnOnce: Send + Sized + 'static {
    type Output: Send + 'static;

    #[doc(hidden)]
    fn into_inner<'a>(self) -> impl AppMutFnOnceInner<'a, Output = Self::Output>;

    /// Call this method by providing an app handle.
    fn call(self, app: &mut App) -> impl AppMutFuture<'_, Output = Self::Output> {
        AppMutFutureImpl {
            app,
            func: self.into_inner(),
        }
    }

    /// Creates a new [`AppMutFnOnce`] that performs this action first, performs the `other` action
    /// right after, and wraps their outputs in a tuple.
    fn then<U: Send + 'static>(
        self,
        other: impl AppMutFnOnce<Output = U>,
    ) -> impl AppMutFnOnce<Output = (Self::Output, U)> {
        Then {
            before: self,
            after: other,
        }
    }

    /// Creates a new [`AppMutFnOnce`] that performs this action and maps its output using the
    /// `mapping`.
    fn map<U: Send + 'static>(
        self,
        mapping: impl FnOnce(Self::Output) -> U + Send + 'static,
    ) -> impl AppMutFnOnce<Output = U> {
        AppFnMapping {
            inner: self,
            mapping,
        }
    }
}

struct Then<B, A> {
    before: B,
    after: A,
}

app_future_def! {
    impl<
        BO: Send + 'static,
        AO: Send + 'static,
        B: @AppFnOnce<Output = BO>,
        A: @AppFnOnce<Output = AO>,
    > @AppFnOnce for Then<B, A>
    {
        type Output = (BO, AO);
        fn into_inner<'a>(self) -> impl @AppFnOnceInner<'a, Output = Self::Output> {
            @AppFnOnceImpl {
                pd: PhantomData,
                source: self,
                handle_fn: move |this, handle| {
                    let before_inner: @AppFnOnceImpl<_, _, _, _, _> =
                        this.before.into_inner().into();
                    let after_inner: @AppFnOnceImpl<_, _, _, _, _> =
                        this.after.into_inner().into();

                    let (before_method, before_fut) =
                        (before_inner.handle_fn)(before_inner.source, handle);
                    let (after_method, after_fut) =
                        (after_inner.handle_fn)(after_inner.source, handle);

                    let joined_fut = futures::FutureExt::then(
                        before_fut,
                        |before_val| futures::FutureExt::map(
                            after_fut,
                            |after_val| (before_val, after_val)
                        )
                    );

                    (
                        Method::Then {
                            before: Box::new(before_method),
                            after: Box::new(after_method),
                        },
                        joined_fut,
                    )
                },
                owned_fn: |this, owned, handle| {
                    let Then { before, after } = this;

                    let before: @AppFnOnceImpl<_, _, _, _, _> = before.into_inner().into();
                    let before_val = (before.owned_fn)(before.source, owned, handle);

                    let after: @AppFnOnceImpl<_, _, _, _, _> = after.into_inner().into();
                    let after_val = (after.owned_fn)(after.source, owned, handle);

                    (before_val, after_val)
                },
            }
        }
    }
}

struct AppFnMapping<A, M> {
    inner: A,
    mapping: M,
}

app_future_def! {
    impl<
        A: @AppFnOnce<Output = O>,
        O: Send + 'static,
        U: Send + 'static,
        M: FnOnce(O) -> U + Send + 'static,
    > @AppFnOnce for AppFnMapping<A, M>
    {
        type Output = U;
        fn into_inner<'a>(self) -> impl @AppFnOnceInner<'a, Output = Self::Output> {
            @AppFnOnceImpl {
                pd: PhantomData,
                source: self,
                handle_fn: move |this, handle| {
                    let inner: @AppFnOnceImpl<_, _, _, _, _> = this.inner.into_inner().into();
                    let (method, fut) = (inner.handle_fn)(inner.source, handle);
                    // WARN: doing this map manually (through an await or something)
                    // causes an ICE for some reason...
                    let fut = futures::future::FutureExt::map(fut, this.mapping);
                    (method, fut)
                },
                owned_fn: |this, owned, handle| {
                    let inner: @AppFnOnceImpl<_, _, _, _, _> = this.inner.into_inner().into();
                    let val = (inner.owned_fn)(inner.source, owned, handle);
                    (this.mapping)(val)
                },
            }
        }
    }
}

app_future_def! {
    #[doc(hidden)]
    pub trait @AppFnOnceInner<'a>: Into<@AppFnOnceImpl<'a, @generics_from_types>> + Send {
        @type_defs;
    }

    struct @AppFnOnceImpl<'a, @generics_bounded> {
        pd: PhantomData<fn(&'a ())>,
        @fields,
    }

    impl<'a, @generics_bounded> @AppFnOnceInner<'a> for @AppFnOnceImpl<'a, @generics_unbounded> {
        @type_impls;
    }
}

pub(super) struct OneshotSender<T>(oneshot::Sender<T>);

impl<T> OneshotSender<T> {
    pub fn send(self, value: T) {
        let _ = self.0.send(value);
    }
}

impl<T> Debug for OneshotSender<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "oneshot::Sender(...)")
    }
}

#[allow(dead_code)] // some immutable functions aren't used
impl AppFn {
    app_future_def! {
        pub(super) const fn @(spawn, spawn_mut)<I: Send + 'static>(
            input: I,
            method: impl FnOnce(I, &HandleData) -> Method + Send + 'static,
            owned: impl FnOnce(I, &@mut Owned, &HandleData)
                + Send
                + 'static,
        ) -> impl @AppFnOnce<Output = ()> {
            struct Spawn<I, M, O> {
                input: I,
                method: M,
                owned: O,
            }

            impl<
                I: Send + 'static,
                M: FnOnce(I, &HandleData) -> Method + Send + 'static,
                O: FnOnce(I, &@mut Owned, &HandleData)
                    + Send
                    + 'static,
            > @AppFnOnce for Spawn<I, M, O>
            {
                type Output = ();
                fn into_inner<'a>(self) -> impl @AppFnOnceInner<'a, Output = Self::Output> {
                    @AppFnOnceImpl {
                        pd: PhantomData,
                        source: self,
                        handle_fn: |this, handle| ((this.method)(this.input, handle), async {}),
                        owned_fn: |this, owned, handle| (this.owned)(this.input, owned, handle),
                    }
                }
            }

            Spawn {
                input,
                method,
                owned,
            }
        }
    }

    app_future_def! {
        pub(super) const fn @(oneshot, oneshot_mut)<I: Send + 'static, O: Send + 'static>(
            input: I,
            method: impl FnOnce(I, OneshotSender<O>, &HandleData)
                -> Method + Send + 'static,
            owned: impl FnOnce(I, &@mut Owned, & HandleData) -> O + Send + 'static,
        ) -> impl @AppFnOnce<Output = O> {
            struct Oneshot<I, M, O> {
                input: I,
                method: M,
                owned: O,
            }

            impl<
                I: Send + 'static,
                Out: Send + 'static,
                M: FnOnce(I, OneshotSender<Out>, &HandleData) -> Method + Send + 'static,
                O: FnOnce(I, &@mut Owned, &HandleData) -> Out
                    + Send
                    + 'static,
            > @AppFnOnce for Oneshot<I, M, O>
            {
                type Output = Out;
                fn into_inner<'a>(self) -> impl @AppFnOnceInner<'a, Output = Self::Output> {
                    @AppFnOnceImpl {
                        pd: PhantomData,
                        source: self,
                        handle_fn: |this, handle| {
                            let (sender, receiver) = oneshot::channel();
                            let sender = OneshotSender(sender);
                            ((this.method)(this.input, sender, handle), async {
                                receiver.await.expect("app shouldn't drop our message")
                            })
                        },
                        owned_fn: |this, owned, handle| (this.owned)(this.input, owned, handle),
                    }
                }
            }

            Oneshot {
                input,
                method,
                owned,
            }
        }
    }

    app_future_def! {
        pub(super) const fn @(using_handle, using_handle_mut)<
            O: Send + 'static,
            F: @AppFnOnce<Output = O>
        >(
            mapping: impl FnOnce(&HandleData) -> F + Send + 'static,
        ) -> impl @AppFnOnce<Output = O> {
            struct UsingHandle<M> {
                mapping: M,
            }

            impl<
                M: FnOnce(&HandleData) -> F + Send + 'static,
                F: @AppFnOnce<Output = O>,
                O: Send + 'static,
            > @AppFnOnce for UsingHandle<M>
            {
                type Output = O;

                fn into_inner<'a>(self) -> impl @AppFnOnceInner<'a, Output = Self::Output> {
                    @AppFnOnceImpl {
                        pd: PhantomData,
                        source: self,
                        handle_fn: |this, handle| {
                            let fn_once: F = (this.mapping)(handle);
                            let inner: @AppFnOnceImpl<_, _, _, _, _>
                                = fn_once.into_inner().into();
                            (inner.handle_fn)(inner.source, handle)
                        },
                        owned_fn: |this, owned, handle| {
                            let fn_once: F = (this.mapping)(handle);
                            let inner: @AppFnOnceImpl<_, _, _, _, _>
                                = fn_once.into_inner().into();
                            (inner.owned_fn)(inner.source, owned, handle)
                        },
                    }
                }
            }

            UsingHandle { mapping }
        }
    }
}
