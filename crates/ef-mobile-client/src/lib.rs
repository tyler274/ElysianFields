//! Curie Mobile Bevy `0.20.1` client built on `ef-ui-catalog` + `ef-mobile-domain` + `ef-core`.
//!
//! Strictly product-isolated from Curie Clinic (`ef-clinic-*` is forbidden by `ef-boundaries`).

pub mod plugin;
pub mod transport;

pub use plugin::{
    CurieMobileClientPlugin, MobileClientCommand, MobileClientCommandQueue, MobileClientState,
    MobileTab, MobileTransportResource, MobileVisibility,
};
pub use transport::{
    LocalMobileTransport, MobileApiEndpoint, MobileClientTransport, MobileTransportError,
};
