use proc_macro::{self, TokenStream};
use quote::quote;
use syn::{parse_macro_input, DeriveInput};

#[proc_macro_derive(CrdtContainer, attributes(crdt))]
pub fn crdt_container(input: TokenStream) -> TokenStream {
    let DeriveInput { ident, data, .. } = parse_macro_input!(input);
    let data = if let syn::Data::Struct(data) = data {
        data
    } else {
        unimplemented!()
    };

    let merges = data.fields.iter().filter_map(|field| {
        if field.attrs.iter().any(|a| a.path().is_ident("crdt")) {
            let ident = if let Some(ident) = &field.ident {
                ident
            } else {
                unimplemented!("Not currently working with unnamed fiends");
            };
            Some(quote! {
                self.#ident.merge(&other.#ident)?;
            })
        } else {
            None
        }
    });

    let cleanups = data.fields.iter().filter_map(|field| {
        if field.attrs.iter().any(|a| a.path().is_ident("crdt")) {
            let ident = if let Some(ident) = &field.ident {
                ident
            } else {
                unimplemented!("Not currently working with unnamed fiends");
            };
            Some(quote! {
                self.#ident.cleanup(now);
            })
        } else {
            None
        }
    });

    let output = quote! {
        impl client_utils::crdt::Crdt for #ident {
            fn merge(&mut self, other: &Self) -> anyhow::Result<()> {
                #(#merges)*
                Ok(())
            }

            fn cleanup(&mut self, now: i64) {
                #(#cleanups)*
            }
        }
    };
    output.into()
}
