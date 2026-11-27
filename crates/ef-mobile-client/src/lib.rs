//! Curie Mobile Bevy `0.20.1` client built on `ef-ui-catalog` + `ef-mobile-domain` + `ef-core`.
//!
//! Strictly product-isolated from Curie Clinic (`ef-clinic-*` is forbidden by `ef-boundaries`).

#[cfg(not(target_arch = "wasm32"))]
pub mod http_transport;
pub mod plugin;
pub mod transport;
pub mod wasm_abi;

#[cfg(not(target_arch = "wasm32"))]
pub use http_transport::{LiveHttpMobileTransport, LiveMobileHttpResponse};
pub use plugin::{
    CurieMobileClientPlugin, ManagedMobileA11yNode, MobileClientCommand, MobileClientCommandQueue,
    MobileClientState, MobileTab, MobileTransportResource, MobileVisibility,
};
pub use transport::{
    AccountDeleteReceipt, ConnectedCareStatusResult, LocalMobileTransport, MobileApiEndpoint,
    MobileClientTransport, MobileTransportError,
};
pub use wasm_abi::{
    BEVY_RUST_WASM_RUNTIME_ID, MOBILE_MOUNT_SUBPATH, MOBILE_WASM_ABI_VERSION,
    MobileWasmA11yNodeDescriptor, MobileWasmCommand, MobileWasmIdleRedrawSummary,
    MobileWasmRuntime, MobileWasmSnapshot, MobileWasmTab, pack_ptr_len, unpack_ptr_len,
};
