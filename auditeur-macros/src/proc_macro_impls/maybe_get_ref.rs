use syn::Ident;


use quote::quote;

pub fn maybe_get_ref(ident: &Ident, generics: &syn::Generics) -> proc_macro2::TokenStream {
    let auditeur = &*crate::AUDITEUR;

    let (impl_generics, type_generics, where_clause) = generics.split_for_impl();

    quote! {
        #[#auditeur::async_trait::async_trait]
        impl #impl_generics #auditeur::input::get::MaybeGetRef for #ident #type_generics #where_clause {
            async fn maybe_get_ref(
                &self,
                registry: &#auditeur::app::Registry,
                dependent: #auditeur::app::ComponentIdRef<'_>,
            ) -> Result<Box<dyn #auditeur::input::get::ResourceRef>, #auditeur::Error> {
                use #auditeur::input::get::maybe_get_ref::Specialization as _;
                (&mut &*self).specialization(registry, dependent).await
            }
        }
    }
}
