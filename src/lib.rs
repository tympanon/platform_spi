use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{quote, quote_spanned, ToTokens};
use syn::{ext::IdentExt, parse::Parse, parse_macro_input, punctuated::Punctuated, spanned::Spanned, token::{Comma, Eq, Semi}, Token};

/// Define a module from a different source file for a specified mapping of compile time attributes.
/// 
/// Each platform-specific implementation must be in a source file named 
/// "{module_path}/{target_os}.rs". {module_path} is "." by default, but may
/// be overridden with an optional "module_path" argument to the macro.
/// 
/// The mapping of attribute to file name is specified with a when clause
/// "when (attribute_name_a, attribute_name_b) [is] {
///                                                      (attribute_value__for_a, attribute_value_for_b) => filename,
///                                                      ...
///                                                 }"
/// The pattern mappings support boolean operations of ! and |.
/// Matching to any value of an attribute with _ is also supported.  Each pattern match is done sequentially 
/// and short circuits if a match is found (unlike with cfg attributes in conditional compilation), so there is 
/// at most one module file defined for any platform being compiled.
/// 
/// ## Aliases
/// Any "type" and "use" declarations in the module content block will be 
/// converted into items in the parent module, which refer to items in the target platform
/// module. These type aliases are the "SPI", required to be implemented
/// for each supported platform. Additionally, an "impl" declaration can be made to specify 
/// that each platform type must implement a specific trait.
/// Item declarations other than "type", "use", and "impl" are not supported.
///
/// ## Examples
/// ```
/// #[platform_spi(when target_os is {
///                                     macos | linux  => unix, 
///                                     windows        => windows,
///                                     _              => unsupported)]
///                                     
/// mod platform {
///     /// A public type alias declared in the parent module. A type named "ServiceImpl<T>" 
///     /// is part of the SPI contract, and must therefore be declared in each source file.
///     pub type PlatformService = ServiceImpl<SomeType>;
/// 
///     /// A platform-specific error type, renamed and exported from the parent module as "PlatformError".
///     pub use ErrorImpl as PlatformError;
/// 
///     /// Trait contract that specifies that each platform-specific PlatformService will implement SomeTrait
///     impl SomeTrait for PlatformService{}
/// }
/// ```
/// 
/// is equivalent to
/// 
/// ```
/// #[cfg(any(target_os = "macos", target_os = "linux")]
/// #[path = "./unix.rs"]
/// mod platform;
/// 
/// #[cfg(target_os = "windows")]
/// #[path = "./windows.rs"]
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

    let (cfgs, file_paths) = match config.mapping.build_module_declarations() {
        Ok(module) => module,
        Err(diagnostics) => return diagnostics,
    };

    // the inline module declaration, rewritten as module file import.
    let mod_import = &rewritten_decl.mod_import_decl;

    // SPI type aliases hoisted from the module declaration.
    let aliases = &rewritten_decl.aliases;

    let (types, impls) = &rewritten_decl.implementations;

    quote! {

        #( 
            #[cfg(#cfgs)]
            #[path = #file_paths]
            #mod_import
        )*

        #(#aliases)*

        #(static_assertions::assert_impl_all!(#types : #impls);)*
    }.into()

}

struct Mapping {
    cfg_attributes: Punctuated::<syn::Ident, Comma>,
    routes: Vec<CustomArm>,
    module_path: syn::LitStr
}

impl Mapping {
    fn build_module_declarations(&self) -> Result<(Vec<TokenStream2>, Vec<String>), TokenStream> {
        let mut result = vec![];
        let mut file_paths = vec![];
        let mut errors = vec![];

        for route in &self.routes {
            let mut pairs = vec![];

            match self.build_cfg_conditions(&mut result, &mut file_paths, route, &self.cfg_attributes, &mut pairs, false) {
                Ok(_) => (),
                Err(err) => errors.push(err),
            }
        }

        if !errors.is_empty() {
            let collected: TokenStream2 = errors.into_iter().collect();
            return Err(collected.into())
        }

        Ok((result, file_paths))
    }

    fn build_cfg_conditions(&self, past_conditions: &mut Vec<TokenStream2>, file_paths: &mut Vec<String>, route: &CustomArm, attributes: &Punctuated::<syn::Ident, Comma>, current_conditions: &mut Vec<TermAttributePair>, disable_interpolation: bool) -> Result<(), TokenStream2> {
        let patterns = &route.pats;
        match (attributes.get(current_conditions.len()), patterns.get(current_conditions.len())) {
            (None, None) => {
                let file_path = self.generate_full_file_path(current_conditions, &route.file_path, disable_interpolation)?;
                let condition = generate_cfg_condition(past_conditions, current_conditions);
                let cfg = quote! {
                    #condition
                };
                past_conditions.push(cfg);
                file_paths.push(file_path);
            },
            //TODO: get spans working with the custom parse objects so errors highlight more accurately
            //TODO: Should try to get string formatting to work so we can see number of patterns and number of attributes (may need to use a different error type)
            //TODO: we should support _ => to match multiple defaults, right now if you have 2 attributes you'd need to specify (_, _) => ...
            (None, Some(_)) => return Err(quote_spanned! {route.file_path.span() => compile_error!("Number of patterns does not match number of attributes.")}),
            (Some(_), None) => return Err(quote_spanned! {route.file_path.span() => compile_error!("Number of patterns does not match number of attributes.")}),
            (Some(attribute), Some(pattern)) => {
                for term in &pattern.terms {
                    current_conditions.push(TermAttributePair{
                        term: term.clone(),
                        attribute: attribute.clone()
                    });
                    match (term.negation, term.val.to_string().as_str()) {
                        (Some(_), "_") => return Err(quote_spanned! {term.val.span() => compile_error!("Negation of _ pattern in platform SPI not valid")}),
                        (Some(_), _) => self.build_cfg_conditions(past_conditions, file_paths, route, attributes, current_conditions, true)?,
                        (None, "_") => self.build_cfg_conditions(past_conditions, file_paths, route, attributes, current_conditions, true)?,
                        _ => self.build_cfg_conditions(past_conditions, file_paths, route, attributes, current_conditions, disable_interpolation)?
                    }
                    current_conditions.pop();
                }
            },
        }
        Ok(())
    }

    fn generate_full_file_path(&self, _current_conditions: &Vec<TermAttributePair>, file_path: &syn::Ident, _disable_interpolation: bool) -> Result<String, TokenStream2> {
        //TODO: interpolation
        Ok(format!("{}/{}.rs", self.module_path.value(), file_path.to_string()))
    }
}



fn generate_cfg_condition(past_conditions: &Vec<TokenStream2>, current_conditions: &Vec<TermAttributePair>) -> TokenStream2 {
    //Note, we take a very naive approach on the mutually exclusive conditions by adding every previous mapping to a not block,
    //which can become large in complex cases
    //TODO: Do something more clever to reduce the size of these boolean expressions and make the macro expansion more readable
    quote! {all(not(any(#(#past_conditions, )*)), #(#current_conditions, )*)}
}

//We create a set of custom parsers due to syn::Pat not supporting ! negation
//We may be able to remove these and use built-in parsers after https://github.com/rust-lang/rust/issues/118155 is implemented
#[derive(Clone)]
struct Term {
    negation: Option<syn::Token![!]>,
    val: syn::Ident
}

impl Parse for Term {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        Ok(Term {
            negation: input.parse()?,
            //use parse_any as _ is a keyword
            //TODO: Right now we only accept unquoted literals.  We should consider supporting quoted strings as terms
            val: input.call(syn::Ident::parse_any)?
        })
    }
}

struct TermAttributePair {
    term: Term,
    attribute: syn::Ident
}

impl ToTokens for TermAttributePair {
    fn to_tokens(&self, tokens: &mut TokenStream2) {
        if self.term.val.to_string().as_str() == "_" {
            //return any match, as a predicate needs to be supplied
            return quote! {any()}.to_tokens(tokens);
        }
        let attribute = &self.attribute;
        let term = &self.term.val.to_string();
        match self.term.negation {
            Some(_) => quote! {not(#attribute = #term)}.to_tokens(tokens),
            None => quote! {#attribute = #term}.to_tokens(tokens),
        }
    }
}


struct CustomPat {
    terms: Vec<Term>
}

impl Parse for CustomPat {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut terms = vec![];
        while !input.is_empty() && !input.peek(syn::Token![=>]) && !input.peek(Comma) {
            terms.push(input.parse()?);
            let _or: Option<syn::Token![|]> = input.parse()?;
        }
        //TODO: throw error if empty
        Ok(CustomPat{terms: terms})
    }
}

struct CustomArm {
    pats: Punctuated::<CustomPat, Comma>,
    _fat_arrow_token: syn::Token![=>],
    file_path: syn::Ident
}

impl Parse for CustomArm {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        Ok(CustomArm {
            pats: if input.peek(syn::token::Paren) {
                let patterns;
                syn::parenthesized!(patterns in input);
                patterns.parse_terminated(CustomPat::parse, Comma)?
            } else {
                let mut punctuated: Punctuated::<CustomPat, Comma> = Default::default();
                punctuated.push(input.parse()?);
                punctuated
            },
            _fat_arrow_token: input.parse()?,

            //TODO: right now we only accept unquotted literals as part of the file path, we should consider supporting quoted strings
            file_path: input.parse()?
        })
    }
}

struct SpiAttributes {
    module_path: syn::LitStr,
    mapping: Mapping
}

impl Parse for SpiAttributes {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        let mut result = SpiAttributes {
            module_path: syn::LitStr::new(".", input.span()),
            mapping: Mapping {
                cfg_attributes: Default::default(),
                routes: Default::default(),
                module_path: syn::LitStr::new(".", input.span())}
        };

        while !input.is_empty() {

            let name = syn::Ident::parse(&input)?;

            match name.to_string().as_str() {
                "module_path" => {
                    let _eq: Eq = input.parse()?;
                    result.module_path = input.parse()?;
                    result.mapping.module_path = result.module_path.clone();
                },
                "when" => {
                    if input.peek(syn::Ident) {
                        //single element
                        let attribute = input.parse()?;
                        result.mapping.cfg_attributes.push(attribute);
                    }
                    else {
                        let attributes;
                        let _parens_token = syn::parenthesized!(attributes in input);
                        result.mapping.cfg_attributes = attributes.parse_terminated(syn::Ident::parse, Comma)?;
                    }
                    let _is: Option<kw::is> = input.parse()?;
                    let arms;
                    let _brace_token = syn::braced!(arms in input);
                    //the patterns can contain commas, so we need to parse the arms one by one rather than with parse_terminated
                    let mut arms_vec = vec![];
                    while !arms.is_empty() {
                        let arm: CustomArm = arms.parse()?;
                        arms_vec.push(arm);
                        let _comma: Option<Comma> = arms.parse()?;
                    }
                    result.mapping.routes = arms_vec;
                }
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

mod kw {
    syn::custom_keyword!(is);
}
