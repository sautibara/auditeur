A trait that allows getting a subset of mutable references out of a tuple.

[`Subset`] is implemented for all tuples starting with `()` and up to `8` other types (see
[#Limitations](Self#limitations)). To select a subset, input a tuple with the same length of
the amount of non-unit types as a generic consisting of only the [`InSet`] and [`OutOfSet`]
types. For each index in the source tuple, if the generic index is an [`InSet`], that element
will be included in the output set, but if it is [`OutOfSet`], the element will not be
included.

# Limitations

Firstly, each input tuple must be prefixed with a `()` to get around the fact that there are no
one-length tuples. If the trait was implemented for sets with only one element (that is, just
the type itself), it would conflict with the other implementations (as other length tuples
would also be seen as "just a type").

Then, since the amount of trait definitions for [`Subset`] grows at an exponential rate as the
amount of accepted inputs increases, only 8 unique types are accepted at this time. To work
around this with inputs, try using a nested tuple or struct with a
[`derive(Input)`](macro@crate::Input) annotation.

Lastly, to limit the amount of trait definitions, the subset must be ordered in the same way as
the elements in the tuple.

# Examples

```rust
use auditeur::input::subset::*;

// this starts with a unit (as required) and has three other types
let mut tuple = ((), 1, true, "hello");
// so, the generic must have three specifiers
// this specifically chooses the second and third type, but not the first one
let (boolean, string) = <_ as Subset<(OutOfSet, InSet, InSet)>>::subset_mut(&mut tuple);
// and so we get the boolean and string out of the tuple
assert_eq!(*boolean, true);
assert_eq!(*string, "hello");
```
