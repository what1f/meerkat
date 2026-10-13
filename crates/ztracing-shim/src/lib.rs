//! Minimal compatibility surface for GPUI's disabled tracing instrumentation.
//!
//! Lankide does not enable Zed's tracing configuration. In that configuration,
//! GPUI only requires an `instrument` attribute that leaves the item unchanged.

use proc_macro::TokenStream;

#[proc_macro_attribute]
pub fn instrument(_: TokenStream, annotated_item: TokenStream) -> TokenStream {
    annotated_item
}
