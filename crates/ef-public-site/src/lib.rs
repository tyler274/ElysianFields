//! Semantic HTML public site, verbatim legal archives, isolated synthetic physician demo,
//! and Dei SAMHSA recovery facility finder (`EF-PUBLIC-0001` & `EF-PUBLIC-0002`).

pub mod dei;
pub mod demo;
pub mod html;
pub mod legal;
pub mod router;
#[cfg(not(target_arch = "wasm32"))]
pub mod server;
pub mod sha256;

pub use dei::{
    DeiCacheError, DeiDisplayMode, DeiSearchQuery, DeiServiceWorkerCache, DeiShardCache,
    DeiShardLoadState, DeiStateShard, RecoveryFacility, ServiceWorkerAssetPair,
    VersionedShardPayload, crisis_lines_appear_before_listings, render_crisis_lines_banner,
    render_dei_index_html, render_dei_state_html, sample_dei_shards,
};
pub use demo::{
    DemoError, DemoTurnOutcome, DemoTurnRecord, SyntheticChartPassage, SyntheticDemoChart,
    SyntheticDemoEngine, VerbatimCitation, VisitorDemoSession,
};
pub use html::{RobotsPolicy, escape_html, render_semantic_page};
pub use legal::{LegalArchiveError, LegalArchiveRegistry, LegalDocument, LegalSection};
pub use router::{
    DeploymentMode, PUBLIC_MAX_BODY_BYTES, PublicIngressGuard, PublicRequest, PublicResponse,
    PublicSiteRouter,
};
#[cfg(not(target_arch = "wasm32"))]
pub use server::TcpPublicSiteServer;
pub use sha256::sha256_hex;
