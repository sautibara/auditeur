use quote::quote;
use syn::Ident;

pub fn resource_ref<'i>(
    ident: &Ident,
    dependency: Option<&'i Ident>,
    fields: impl Iterator<Item = &'i Ident>,
) -> proc_macro2::TokenStream {
    let auditeur = &*crate::AUDITEUR;

    let (message, event_sender) = dependency.map_or_else(Default::default, |dependency| {
        (
            quote! {
                self.event_sender.send_dependency(
                    dependent,
                    <#dependency as #auditeur::resource::Identifiable>::const_ident(),
                );
            },
            quote! {
                event_sender: self.event_sender.clone(),
            },
        )
    });

    quote! {
        impl #auditeur::input::get::ResourceRef for #ident {
            fn clone_as_boxed(
                &self,
                dependent: &#auditeur::namespaced_id::NamespacedIdRef,
            ) -> Box<dyn #auditeur::input::get::ResourceRef> {
                Box::new(self.clone_as(dependent))
            }

            fn clone_as(&self, dependent: &#auditeur::namespaced_id::NamespacedIdRef) -> Self
            where
                Self: Sized
            {
                #message
                use #auditeur::input::get::resource_ref::Specialization as _;
                Self {
                    #event_sender
                    #(#fields: (&mut &self.#fields).specialization(dependent)),*
                }
            }
        }
    }
}
