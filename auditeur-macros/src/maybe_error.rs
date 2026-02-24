pub type MaybeError = Option<syn::Error>;
pub type Continue<T> = (T, MaybeError);

pub trait MaybeErrorExt {
    fn combine(&mut self, error: syn::Error);
    fn combine_maybe(&mut self, error: MaybeError);
    fn wrap<T>(self, value: T) -> Continue<T>;
}

impl MaybeErrorExt for MaybeError {
    fn combine(&mut self, error: syn::Error) {
        if let Some(old) = self.as_mut() {
            old.combine(error);
        } else {
            *self = Some(error);
        }
    }

    fn combine_maybe(&mut self, error: MaybeError) {
        if let Some(error) = error {
            self.combine(error);
        }
    }

    fn wrap<T>(self, value: T) -> Continue<T> {
        (value, self)
    }
}

pub trait ContinueExt<T> {
    fn drain_into(self, error: &mut MaybeError) -> T;
}

impl<T> ContinueExt<T> for Continue<T> {
    fn drain_into(self, error: &mut MaybeError) -> T {
        let (val, err) = self;
        error.combine_maybe(err);
        val
    }
}

pub fn drain_results_into<T>(combine: &mut MaybeError) -> impl FnMut(syn::Result<T>) -> Option<T> {
    |res| match res {
        Ok(val) => Some(val),
        Err(err) => {
            combine.combine(err);
            None
        }
    }
}

pub fn drain_continues_into<T>(combine: &mut MaybeError) -> impl FnMut(Continue<T>) -> T {
    |(val, err)| {
        combine.combine_maybe(err);
        val
    }
}
