//! Curie Clinic Bevy `0.20.1` vertical-slice client.
//!
//! Built on `ef-ui-catalog`, `ef-clinic-domain`, `ef-harness`, and `ef-core` with zero
//! client-side secrets and zero duplicate authoritative clinical stores.

#[cfg(not(target_arch = "wasm32"))]
pub mod http_transport;
pub mod plugin;
pub mod state;
pub mod transport;
pub mod wire;

#[cfg(not(target_arch = "wasm32"))]
pub use http_transport::{LiveHttpClinicTransport, LiveHttpResponse};
pub use plugin::{
    ClientFrameReport, ClinicAction, ClinicActionMessage, ClinicTransportResource,
    CurieClinicClientPlugin, DraftConflictChoice, ManagedClinicA11yNode, SignInEntryMethod,
};
pub use state::{
    ActiveWorkspaceView, BrowserSessionHubInner, ClientClock, ClientRuntimeConfig,
    ClinicSessionShell, DraftBannerState, OpenPatientWorkspace, OriginalLanguageNotice,
    SharedBrowserSessionHub, SharedLocaleStorage, SignInFormStatus, StreamApplyEffect,
    TodayRosterView, core_to_ui_locale, ui_to_core_locale,
};
pub use transport::{
    ClinicTransport, LocalVerticalSliceTransport, TransportError, TransportResponseMeta,
    VerticalSliceBackend, stream_event_id,
};
pub use wire::{
    ChatSnapshotWire, ClientBootConfig, ClinicEndpoint, PatientWorkspaceDetail, PutDraftRequest,
    STRICT_WASM_CSP, SignInRequest, SseWireError, VerifySignInRequest, decode_sse_frame,
    encode_sse_frame, is_strict_wasm_csp,
};
