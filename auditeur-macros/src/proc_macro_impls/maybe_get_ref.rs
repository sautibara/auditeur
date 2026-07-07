use syn::Ident;

use quote::quote;

pub fn maybe_get_ref(ident: &Ident, generics: &syn::Generics) -> proc_macro2::TokenStream {
    let auditeur = &*crate::AUDITEUR;

    let (impl_generics, type_generics, where_clause) = generics.split_for_impl();

    quote! {
        impl #impl_generics #auditeur::input::get::MaybeGetRef for #ident #type_generics
        #where_clause
        {
            async fn maybe_get_ref(
                &self,
                app: &mut #auditeur::app::App,
            ) -> Result<
                Option<Box<dyn #auditeur::input::get::DynGetRefLoader>>,
                #auditeur::Error
            > {
                use #auditeur::input::get::maybe_get_ref::Specialization as _;
                (&mut &*self).specialization(app).await
            }
        }
    }
}
