#![doc = include_str!("subset_docs.md")]

use auditeur_macros::all_combinations;
use bevy_utils_proc_macros::all_tuples;

#[doc = include_str!("subset_docs.md")]
pub trait Subset<T> {
    /// A tuple of mutable references to each item in this subset.
    type SubsetMut<'s>
    where
        Self: 's;

    /// Obtain a tuple of mutable references to each item in this subset.
    ///
    /// # Examples
    ///
    /// See the type docs example.
    fn subset_mut(&mut self) -> Self::SubsetMut<'_>;
}

/// A unit struct that instructs [`Subset`] to include the type in this position in the subset.
pub struct InSet;
/// A unit struct that instructs [`Subset`] to not include the type in this position in the subset.
pub struct OutOfSet;

impl<T: 'static> Subset<()> for T {
    type SubsetMut<'s> = ();
    fn subset_mut(&mut self) -> Self::SubsetMut<'_> {}
}

macro_rules! impl_subset {
    ([$set_generics:ty, ($($pattern:pat),*), $result:expr, ($($included_type:ty),*)] ($($generic_type:tt),*)) => {
        #[allow(unused_parens)]
        impl<$($generic_type),*> Subset<$set_generics> for ((), $($generic_type),*)
        where $($generic_type: 'static),*
        {
            type SubsetMut<'s> = ($(&'s mut $included_type),*);
            fn subset_mut(&mut self) -> Self::SubsetMut<'_> {
                let ((), $($pattern),*) = self;
                $result
            }
        }
    };
}

macro_rules! impl_subsets {
    ($(($T:ident, $t:ident)),*) => {
        all_combinations!(
            macro_ident = impl_subset,
            rules = [
                (
                    in_set = Just(InSet),
                    out_of_set = Just(OutOfSet),
                ),
                (
                    in_set = List[$($t),*],
                    out_of_set = Just(_),
                ),
                (
                    in_set = List[$($t),*],
                    out_of_set = None,
                ),
                (
                    in_set = List[$($T),*],
                    out_of_set = None,
                ),
            ],
            passthru = ($($T),*)
        );
    };
}

all_tuples!(impl_subsets, 1, 8, T, t);
