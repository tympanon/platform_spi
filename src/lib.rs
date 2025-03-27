use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{quote, quote_spanned, ToTokens};
use syn::{bracketed, parse::Parse, parse_macro_input, punctuated::Punctuated, spanned::Spanned, token::{Comma, Eq, Semi}, Token};

/// Define a module from a different source file for each named target OS.
/// 
/// Each platform-specific implementation must be in a source file named 
/// "{module_path}/{target_os}.rs". {module_path} is "." by default, but may
/// be overridden with an optional "module_path" argument to the macro.
/// 
/// ## Aliases
/// Any "type" and "use" declarations in the module content block will be 
/// converted into items in the parent module, which refer to items in the target platform
/// module. These type aliases are the "SPI", required to be implemented
/// for each supported platform. Additionally, an "impl" declaration can be made to specify 
/// that each platform type must implement a specific trait.
/// Item declarations other than "type", "use", and "impl" are not supported.
/// 
/// ## Unsupported Platforms
/// One additional source file, "unsupported.rs", will be used for attempted compilation 
/// on any unsupported target platform. Note that it is not necessary to actually 
/// create unsupported.rs if you never intend to build for an unsupported platform.
/// 
/// ## Examples
/// ```
/// #[platform_spi(targets = [macos, windows, linux])]
/// mod platform {
///     /// A public type alias declared in the parent module. A type named "ServiceImpl<T>" 
///     /// is part of the SPI contract, and must therefore be declared in each source file.
///     pub type PlatformService = ServiceImpl<SomeType>;
/// 
///     /// A platform-specific error type, renamed and exported from the parent module as "PlatformError".
///     pub use ErrorImpl as PlatformError;
/// 
///     /// Trait contract that specifies that each platform-specific PlatformService will implement SomeTrait
///     impl SomeTrait for PlatformService;
/// }
/// ```
/// 
/// is equivalent to
/// 
/// ```
/// #[cfg(target_os = "macos")]
/// #[path = "./macos.rs"]
/// mod platform;
/// 
/// #[cfg(target_os = "windows")]
/// #[path = "./windows.rs"]
/// mod platform;
/// 
/// #[cfg(target_os = "linux")]
/// #[path = "./linux.rs"]
/// mod platform;
/// 
/// #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
/// #[path = "./unsupported.rs"]
/// mod platform;
/// 
/// #[doc = "Public type alias to the platform-specific implementation of trait Service<T>."]
/// pub type PlatformService<T> = platform::ServiceImpl<T>;
/// #[doc = "A platform-specific error type, renamed and exported from the parent module as \"PlatformError\"."]
/// pub use platform::ErrorImpl as PlatformError;
/// 
/// static_assertions::assert_impl_all!(PlatformService : SomeTrait);
/// ```
#[proc_macro_attribute]
pub fn platform_spi(args: TokenStream, item: TokenStream) -> TokenStream {

    let config = parse_macro_input!(args as SpiAttributes);

    let mod_decl = parse_macro_input!(item as syn::ItemMod);
    let rewritten_decl = match SpiModule::try_from(&mod_decl) {
        Ok(module) => module,
        Err(diagnostics) => return diagnostics,
    };

    // the inline module declaration, rewritten as module file import.
    let mod_import = &rewritten_decl.mod_import_decl;

    let target_names: Vec<String> = config.target_names();
    let mod_paths: Vec<String> = config.source_paths();

    // SPI type aliases hoisted from the module declaration.
    let aliases = &rewritten_decl.aliases;

    let (types, impls) = &rewritten_decl.implementations;

    quote! {
        #( 
            #[cfg(target_os = #target_names)]
            #[path = #mod_paths]
            #mod_import
        )*

        #[cfg(not(any(#( target_os = #target_names ),*)))]
        #[path = "./unsupported.rs"]
        #mod_import

        #(#aliases)*

        #(static_assertions::assert_impl_all!(#types : #impls);)*
    }.into()

}

struct SpiAttributes {
    targets: Punctuated::<syn::Ident, Comma>,
    module_path: syn::LitStr
}
impl SpiAttributes {
    // string literals naming each module source file, e.g. "./macos.rs"
    fn source_paths(&self) -> Vec<String> {
        self.targets.iter().map(
            |id| format!("{}/{id}.rs", self.module_path.value())
        ).collect()
    }

    /// string literals naming each target_os value, e.g. "macos"
    fn target_names(&self) -> Vec<String> {
        self.targets.iter().map(syn::Ident::to_string).collect()
    }
}
impl Parse for SpiAttributes {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut result = SpiAttributes {
            module_path: syn::LitStr::new(".", input.span()),
            targets: Default::default()
        };

        while !input.is_empty() {

            let name = syn::Ident::parse(&input)?;
            let _eq: Eq = input.parse()?;

            match name.to_string().as_str() {
                "targets" => {
                    let targets;
                    let _bracket = bracketed!(targets in input);
                    result.targets = targets.parse_terminated(syn::Ident::parse, Comma)?;
                },
                "module_path" => {
                    result.module_path = input.parse()?
                },
                _ => return Err(input.error(format!("Unexpected attribute '{name}'")))
            }

            let _comma = input.lookahead1();
            if _comma.peek(Comma) {
                let _comma: Comma = input.parse()?;
            }
        }

        Ok(result)
    }
}

struct SpiModule {
    mod_import_decl: syn::ItemMod,
    aliases: Vec<syn::Item>,
    implementations: (Vec<syn::Type>, Vec<syn::Path>)
}
// implementing TryFrom rather than Parse allows us to reuse most of the parse logic
// from ItemMod, plus be a little more fine-grained with errors (e.g. we can report 
// multiple errors, limit our errors to specific spans).
impl TryFrom<&syn::ItemMod> for SpiModule {
    type Error = TokenStream;

    fn try_from(mod_decl: &syn::ItemMod) -> Result<Self, Self::Error> {
        let parent_module = mod_decl.ident.clone();

        let mod_aliases = check_spi_items(mod_decl)?;
        let (aliases, implementations) = hoist_aliases_and_generate_impls(mod_aliases, parent_module)?;

        let mod_import_decl = syn::ItemMod {
            attrs: mod_decl.attrs.clone(),
            vis: mod_decl.vis.clone(),
            unsafety: mod_decl.unsafety.clone(),
            mod_token: mod_decl.mod_token,
            ident: mod_decl.ident.clone(),
            content: None,
            semi: Some(Semi(mod_decl.ident.span())),
        };

        Ok(Self { mod_import_decl, aliases, implementations})
    }
}

fn check_spi_items(mod_decl: &syn::ItemMod) -> Result<&[syn::Item], TokenStream> {
    match &mod_decl.content {
        Some((_, content)) => 
            Ok(content),
        None => 
            Err(quote_spanned! {
                mod_decl.ident.span() => 
                    compile_error!("External module imports are not supported, only inline module declarations.")
            }.to_token_stream().into())
    }
}

fn hoist_aliases_and_generate_impls(mod_aliases: &[syn::Item], parent_module: syn::Ident) -> Result<(Vec<syn::Item>, (Vec<syn::Type>, Vec<syn::Path>)), TokenStream> {
    let mut invalid_items: Vec<TokenStream2> = vec![];
    let mut aliases: Vec<syn::Item> = vec![];
    let mut impl_types: Vec<syn::Type> = vec![];
    let mut impls: Vec<syn::Path> = vec![];

    for item in mod_aliases {
        if let syn::Item::Impl(impl_item) = item {
            if let (0, None, Some((None, path, _))) = (impl_item.items.len(), &impl_item.generics.where_clause, &impl_item.trait_) {
                impl_types.push(*impl_item.self_ty.clone());
                impls.push(path.clone());
            } else {
                invalid_items.push(quote_spanned! {
                    item.span() => compile_error!("Impl block is incorrectly formed, only format of 'impl Trait for Type {}' is allowed")
                });
            }
            continue;
        }
        let hoisted = match item {
            syn::Item::Type(alias) => hoist_type_alias(alias, &parent_module),
            syn::Item::Use(alias) => hoist_use_alias(alias, &parent_module),
            _ => Err(quote_spanned! {
                item.span() => compile_error!("Only 'type', 'use', and 'impl' items are supported in an SPI module declaration but found")
            })
        };
        match hoisted {
            Ok(item) => aliases.push(item),
            Err(diagnostic) => invalid_items.push(diagnostic),
        }
    }

    if !invalid_items.is_empty() {
        let collected: TokenStream2 = invalid_items.into_iter().collect();
        return Err(collected.into())
    }

    Ok((aliases, (impl_types, impls)))
}

fn hoist_type_alias(alias: &syn::ItemType, parent_module: &syn::Ident) -> Result<syn::Item, TokenStream2> {
    match alias.ty.as_ref() {
        syn::Type::Path(type_path) => {
            let parent_path = syn::PathSegment {
                ident: parent_module.clone(),
                arguments: syn::PathArguments::None
            };
            let mut hoisted_path = type_path.clone();
            hoisted_path.path.segments.insert(0, parent_path);

            let mut hoisted = alias.clone();
            hoisted.ty = Box::new(syn::Type::Path(hoisted_path));

            Ok(syn::Item::Type(hoisted))
        },
        _ => {
            Err(quote_spanned! {
                alias.span() => compile_error!("Only path aliases are supported in an SPI module declaration")
            })
        }
    }
}

fn hoist_use_alias(alias: &syn::ItemUse, parent_module: &syn::Ident) -> Result<syn::Item, TokenStream2> {
    let mut hoisted = alias.clone();
    hoisted.tree = syn::UseTree::Path(syn::UsePath {
        ident: parent_module.clone(),
        colon2_token: Token![::](alias.span()),
        tree: Box::new(hoisted.tree)
    });
    Ok(syn::Item::Use(hoisted))
}
