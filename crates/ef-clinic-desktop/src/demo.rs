//! Synthetic demo workspace seeder and live server bootstrap for Curie Clinic Desktop.

use std::sync::{Arc, Mutex};

use bevy::prelude::Resource;
use ef_clinic_client::{
    ClinicAction, ClinicActionMessage, ClinicSessionShell, ClinicTransport,
    ClinicTransportResource, LocalVerticalSliceTransport, SignInEntryMethod, SignInFormStatus,
    TodayRosterView, VerticalSliceBackend,
};
use ef_clinic_domain::auth::{
    AccountStanding, EmailAddress, PhysicianAccount, PhysicianRole, SignInMode,
};
use ef_clinic_domain::chat::RetryBackoffMode;
use ef_clinic_domain::workspace::{
    CitationOrigin, CitedSourcePayload, PatientRosterState, ReadablePart, classify_record_status,
};
use ef_core::{ChatId, Locale as CoreLocale, PhysicianId, PracticeId, SourceKey, WorkspaceId};
use ef_harness::StubProvider;

use crate::config::{DesktopLaunchConfig, LaunchBackend};
use crate::worker::LiveSubmissionState;

/// Realistic synthetic progressive streaming draft seeded by `--streaming-demo`.
pub const STREAMING_DEMO_DRAFT: &str = "HbA1c improved from 7.4% to 6.8% on Metformin 1000 mg BID; renal panel eGFR > 60 mL/min [1]...";

/// Synthetic streaming step label paired with `--streaming-demo`.
pub const STREAMING_DEMO_STEP: &str = "Drafting response...";

/// Display metadata for one synthetic or server-backed patient in the desktop roster rail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopPatientEntry {
    pub workspace_id: WorkspaceId,
    pub display_name: String,
    pub summary_line: String,
}

/// Display metadata for one cited clinical source shown in the Evidence rail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopCitationCard {
    pub source_key: SourceKey,
    pub number: u16,
    pub label: String,
    pub title: String,
    pub excerpt: String,
    pub provenance: String,
}

/// Single rendered turn line in the desktop transcript panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopTranscriptEntry {
    pub speaker: String,
    pub body: String,
}

/// Active connection posture displayed in the desktop workspace header/rail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionMode {
    DemoLocal,
    LiveServer { base_url: String },
}

impl ConnectionMode {
    #[must_use]
    pub fn badge_text(&self) -> String {
        match self {
            Self::DemoLocal => "MODE: DEMO (LOCAL)".to_owned(),
            Self::LiveServer { base_url } => format!("SERVER: {base_url}"),
        }
    }

    #[must_use]
    pub fn localized_badge(&self, locale: ef_ui_catalog::Locale) -> String {
        match self {
            Self::DemoLocal => {
                ef_ui_catalog::translate(locale, ef_ui_catalog::UiKey::BadgeModeDemoLocal)
                    .to_owned()
            }
            Self::LiveServer { base_url } => format!(
                "{}: {base_url}",
                ef_ui_catalog::translate(locale, ef_ui_catalog::UiKey::BadgeModeLiveServer)
            ),
        }
    }
}

/// Explicit active keyboard input focus target in Curie Clinic Desktop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DesktopInputFocus {
    #[default]
    Composer,
    PatientSearch,
}

/// Typed status banner state rendered reactively in the active 12-locale catalog.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum DesktopStatusBanner {
    #[default]
    DefaultReady,
    Sending,
    Streaming(Option<String>),
    Cancelled,
    Copied,
    PrivacyDismissed,
    SendFailed(String),
    Custom(String),
}

/// Presentation-layer state owned by the native Bevy desktop shell alongside `CurieClinicClientPlugin`.
#[derive(Debug, Clone, Resource)]
pub struct DesktopWorkspaceState {
    pub connection_mode: ConnectionMode,
    pub patients: Vec<DesktopPatientEntry>,
    pub active_patient_index: usize,
    pub citations: Vec<DesktopCitationCard>,
    pub selected_citation_index: usize,
    pub transcript: Vec<DesktopTranscriptEntry>,
    /// Progressive streaming assistant draft accumulated live as SSE `DeltaChunk` frames arrive.
    pub streaming_draft: Option<String>,
    /// Most recent prompt text held while in flight or failed so Retry can re-submit cleanly.
    pub last_failed_prompt: Option<String>,
    pub status_message: String,
    pub status_override: Option<String>,
    pub status_banner: DesktopStatusBanner,
    pub input_focus: DesktopInputFocus,
    pub patient_search_query: String,
    pub search_highlight_index: usize,
    pub live_submission: LiveSubmissionState,
    pub next_live_request_id: u64,
    pub shortcuts_modal_open: bool,
    pub zoom_percent: u16,
    pub frames_rendered_ui: u64,
    pub prompts_submitted: u64,
}

impl DesktopWorkspaceState {
    #[must_use]
    pub fn active_patient(&self) -> Option<&DesktopPatientEntry> {
        self.patients.get(self.active_patient_index)
    }

    #[must_use]
    pub fn selected_citation(&self) -> Option<&DesktopCitationCard> {
        self.citations.get(self.selected_citation_index)
    }

    /// Returns indices into `self.patients` matching `patient_search_query` case-insensitively
    /// across patient display name and clinical summary.
    #[must_use]
    pub fn filtered_patient_indices(&self) -> Vec<usize> {
        let needle = self.patient_search_query.trim().to_ascii_lowercase();
        if needle.is_empty() {
            return (0..self.patients.len()).collect();
        }
        self.patients
            .iter()
            .enumerate()
            .filter(|(_, p)| {
                p.display_name.to_ascii_lowercase().contains(&needle)
                    || p.summary_line.to_ascii_lowercase().contains(&needle)
            })
            .map(|(idx, _)| idx)
            .collect()
    }

    /// Clamps `search_highlight_index` to the bounds of the current filtered patient list.
    pub fn clamp_search_highlight(&mut self) {
        let count = self.filtered_patient_indices().len();
        if count == 0 {
            self.search_highlight_index = 0;
        } else if self.search_highlight_index >= count {
            self.search_highlight_index = count - 1;
        }
    }

    /// Formats the active clinical transcript for clipboard copy.
    #[must_use]
    pub fn transcript_copy_text(&self) -> String {
        self.transcript
            .iter()
            .map(|entry| format!("[{}] {}", entry.speaker, entry.body))
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    /// Applies deterministic CLI preview flags (`--search`, `--streaming-demo`) using a single owner.
    pub fn apply_preview_config(&mut self, config: &DesktopLaunchConfig) {
        if let Some(ref query) = config.search_query {
            self.input_focus = DesktopInputFocus::PatientSearch;
            self.patient_search_query.clone_from(query);
            self.search_highlight_index = 0;
            self.clamp_search_highlight();
        }
        if config.streaming_demo {
            let workspace_id = self
                .active_patient()
                .map(|p| p.workspace_id.clone())
                .unwrap_or_else(|| {
                    WorkspaceId::new("ws-avery-jordan").unwrap_or_else(|_| unreachable!("id"))
                });
            let request_id = self.next_live_request_id;
            self.next_live_request_id = self.next_live_request_id.saturating_add(1);
            self.live_submission = LiveSubmissionState::InFlight {
                request_id,
                workspace_id,
            };
            self.streaming_draft = Some(STREAMING_DEMO_DRAFT.to_owned());
            self.status_override = None;
            self.status_banner =
                DesktopStatusBanner::Streaming(Some(STREAMING_DEMO_STEP.to_owned()));
        }
    }
}

/// Seeded demo transport plus initial synthetic metadata.
pub struct DemoSeedBundle {
    pub transport: LocalVerticalSliceTransport,
    pub clinician_email: String,
    pub patients: Vec<DesktopPatientEntry>,
    pub citations_by_workspace: Vec<(WorkspaceId, Vec<DesktopCitationCard>)>,
}

/// Builds the rich zero-setup local vertical-slice demo backend (`Avery, Jordan`, `Patel, Priya`, `Cohen, Avi`).
#[must_use]
pub fn build_demo_seed() -> DemoSeedBundle {
    let practice_id =
        PracticeId::new("prac-desktop-demo").unwrap_or_else(|_| unreachable!("static id"));
    let physician_id =
        PhysicianId::new("phys-desktop-demo").unwrap_or_else(|_| unreachable!("static id"));
    let clinician_email = "clinician@practice.example".to_owned();

    let ws_avery =
        WorkspaceId::new("ws-avery-jordan").unwrap_or_else(|_| unreachable!("static id"));
    let ws_patel = WorkspaceId::new("ws-patel-priya").unwrap_or_else(|_| unreachable!("static id"));
    let ws_cohen = WorkspaceId::new("ws-cohen-avi").unwrap_or_else(|_| unreachable!("static id"));

    let chat_avery = ChatId::new("chat-avery-1").unwrap_or_else(|_| unreachable!("static id"));
    let chat_patel = ChatId::new("chat-patel-1").unwrap_or_else(|_| unreachable!("static id"));
    let chat_cohen = ChatId::new("chat-cohen-1").unwrap_or_else(|_| unreachable!("static id"));

    let src_avery_1 = SourceKey::new("src-avery-note-1").unwrap_or_else(|_| unreachable!("id"));
    let src_avery_2 = SourceKey::new("src-avery-labs-2").unwrap_or_else(|_| unreachable!("id"));
    let src_patel_1 = SourceKey::new("src-patel-note-1").unwrap_or_else(|_| unreachable!("id"));
    let src_cohen_1 = SourceKey::new("src-cohen-note-1").unwrap_or_else(|_| unreachable!("id"));

    let stub = StubProvider::clinical_two_round_fixture(src_avery_1.clone());
    let mut backend = VerticalSliceBackend::new(
        SignInMode::LocalProof,
        RetryBackoffMode::DeterministicTest,
        stub,
    );

    backend.auth.upsert_account(PhysicianAccount {
        physician_id: physician_id.clone(),
        practice_id: practice_id.clone(),
        email: EmailAddress::parse(&clinician_email).unwrap_or_else(|_| unreachable!("email")),
        display_name: "Dr. Curie Synthetic".to_owned(),
        role: PhysicianRole::Standard,
        standing: AccountStanding::Active,
        sessions_ended_before: None,
    });

    backend.evidence.register_workspace(
        ws_avery.clone(),
        practice_id.clone(),
        "Avery, Jordan",
        vec![physician_id.clone()],
    );
    backend.evidence.register_workspace(
        ws_patel.clone(),
        practice_id.clone(),
        "Patel, Priya",
        vec![physician_id.clone()],
    );
    backend.evidence.register_workspace(
        ws_cohen.clone(),
        practice_id,
        "Cohen, Avi",
        vec![physician_id],
    );

    backend
        .default_chats
        .insert(ws_avery.clone(), chat_avery.clone());
    backend
        .default_chats
        .insert(ws_patel.clone(), chat_patel.clone());
    backend
        .default_chats
        .insert(ws_cohen.clone(), chat_cohen.clone());

    backend.synopses.insert(
        ws_avery.clone(),
        "Glycemic & BP follow-up: HbA1c 6.8%, BP 122/78 mmHg; Metformin 1000 mg BID, Lisinopril 10 mg daily.".to_owned(),
    );
    backend.synopses.insert(
        ws_patel.clone(),
        "Cardiovascular prevention: LDL 84 mg/dL, BP 118/74 mmHg; Atorvastatin 20 mg nightly."
            .to_owned(),
    );
    backend.synopses.insert(
        ws_cohen.clone(),
        "Renal & endocrine review: eGFR 74 mL/min/1.73m2, HbA1c 6.8%, BP 122/78 mmHg.".to_owned(),
    );

    let avery_sources = [
        make_source(
            src_avery_1.clone(),
            ws_avery.clone(),
            1,
            "Progress Note [1]",
            "Outpatient Glycemic Follow-Up Note",
            "progress_note",
            "2026-10-18T14:20:00Z",
            "HbA1c 6.8% (down from 7.2%), home BP 122/78 mmHg. Active medications: Metformin 1000 mg BID, Lisinopril 10 mg daily.",
        ),
        make_source(
            src_avery_2.clone(),
            ws_avery.clone(),
            2,
            "Metabolic Panel [2]",
            "Comprehensive Metabolic & HbA1c Panel",
            "laboratory",
            "2026-10-16T08:45:00Z",
            "HbA1c 6.8%, Creatinine 0.94 mg/dL, eGFR 82 mL/min/1.73m2, Potassium 4.2 mmol/L.",
        ),
    ];
    let patel_sources = [make_source(
        src_patel_1.clone(),
        ws_patel.clone(),
        1,
        "Lipid & BP Note [1]",
        "Preventive Cardiology Review",
        "progress_note",
        "2026-10-12T10:15:00Z",
        "LDL-C 84 mg/dL, HDL-C 58 mg/dL, BP 118/74 mmHg. Active medication: Atorvastatin 20 mg PO QHS.",
    )];
    let cohen_sources = [make_source(
        src_cohen_1.clone(),
        ws_cohen.clone(),
        1,
        "Nephrology Note [1]",
        "Renal & Endocrine Consultation",
        "progress_note",
        "2026-10-10T11:30:00Z",
        "eGFR 74 mL/min/1.73m2, Urine ACR 18 mg/g, HbA1c 6.8%, BP 122/78 mmHg. Continue ACE inhibitor.",
    )];

    for src in avery_sources
        .iter()
        .chain(patel_sources.iter())
        .chain(cohen_sources.iter())
    {
        backend.evidence.insert_source(src.clone());
    }

    let patients = vec![
        DesktopPatientEntry {
            workspace_id: ws_avery.clone(),
            display_name: "Avery, Jordan".to_owned(),
            summary_line: "HbA1c 6.8% | BP 122/78 mmHg | Metformin, Lisinopril".to_owned(),
        },
        DesktopPatientEntry {
            workspace_id: ws_patel.clone(),
            display_name: "Patel, Priya".to_owned(),
            summary_line: "LDL 84 mg/dL | BP 118/74 mmHg | Atorvastatin".to_owned(),
        },
        DesktopPatientEntry {
            workspace_id: ws_cohen.clone(),
            display_name: "Cohen, Avi".to_owned(),
            summary_line: "eGFR 74 | HbA1c 6.8% | BP 122/78 mmHg".to_owned(),
        },
    ];

    let citations_by_workspace = vec![
        (ws_avery, avery_sources.iter().map(source_to_card).collect()),
        (ws_patel, patel_sources.iter().map(source_to_card).collect()),
        (ws_cohen, cohen_sources.iter().map(source_to_card).collect()),
    ];

    let transport = LocalVerticalSliceTransport::new(Arc::new(Mutex::new(backend)));
    DemoSeedBundle {
        transport,
        clinician_email,
        patients,
        citations_by_workspace,
    }
}

#[allow(clippy::too_many_arguments)]
fn make_source(
    source_key: SourceKey,
    workspace_id: WorkspaceId,
    number: u16,
    source_label: &str,
    title: &str,
    record_type: &str,
    recorded_at: &str,
    quote: &str,
) -> CitedSourcePayload {
    CitedSourcePayload {
        source_key,
        workspace_id,
        number,
        source_label: source_label.to_owned(),
        title: title.to_owned(),
        record_type: record_type.to_owned(),
        recorded_at: recorded_at.to_owned(),
        record_status: "final".to_owned(),
        status_class: classify_record_status("final"),
        origin: CitationOrigin::SourceRecord,
        original_language: CoreLocale::En,
        quote: quote.to_owned(),
        body: quote.to_owned(),
        readable_parts: vec![ReadablePart {
            label: "Clinical Summary".to_owned(),
            text: quote.to_owned(),
            unreadable_notice: None,
        }],
        fields: Vec::new(),
        raw_content: None,
        external_url: None,
    }
}

fn source_to_card(src: &CitedSourcePayload) -> DesktopCitationCard {
    DesktopCitationCard {
        source_key: src.source_key.clone(),
        number: src.number,
        label: src.source_label.clone(),
        title: src.title.clone(),
        excerpt: src.quote.clone(),
        provenance: format!(
            "{} | {} | {}",
            src.record_type, src.record_status, src.recorded_at
        ),
    }
}

/// Resource storing workspace-keyed citation cards for instant desktop rail refresh.
#[derive(Debug, Clone, Resource, Default)]
pub struct DesktopCitationCatalog {
    pub by_workspace: Vec<(WorkspaceId, Vec<DesktopCitationCard>)>,
}

impl DesktopCitationCatalog {
    #[must_use]
    pub fn cards_for(&self, ws_id: &WorkspaceId) -> Vec<DesktopCitationCard> {
        self.by_workspace
            .iter()
            .find(|(id, _)| id == ws_id)
            .map(|(_, cards)| cards.clone())
            .unwrap_or_default()
    }
}

/// Builds the typed [`ClinicTransportResource`] and initial [`DesktopWorkspaceState`] for either
/// `--demo` or `--server <url>`.
pub fn create_transport_for_backend(
    backend: &LaunchBackend,
) -> (
    ClinicTransportResource,
    ConnectionMode,
    Option<String>,
    Option<String>,
    Vec<DesktopPatientEntry>,
    DesktopCitationCatalog,
) {
    match backend {
        LaunchBackend::Demo => {
            let seed = build_demo_seed();
            (
                ClinicTransportResource::new(seed.transport),
                ConnectionMode::DemoLocal,
                Some(seed.clinician_email),
                None,
                seed.patients,
                DesktopCitationCatalog {
                    by_workspace: seed.citations_by_workspace,
                },
            )
        }
        LaunchBackend::Server {
            base_url,
            socket_addr,
            email,
            token,
        } => {
            #[cfg(not(target_arch = "wasm32"))]
            {
                let transport = ef_clinic_client::LiveHttpClinicTransport::new(*socket_addr);
                (
                    ClinicTransportResource::new(transport),
                    ConnectionMode::LiveServer {
                        base_url: base_url.clone(),
                    },
                    email.clone(),
                    token.clone(),
                    Vec::new(),
                    DesktopCitationCatalog::default(),
                )
            }
            #[cfg(target_arch = "wasm32")]
            {
                let _ = (base_url, socket_addr, email, token);
                let seed = build_demo_seed();
                (
                    ClinicTransportResource::new(seed.transport),
                    ConnectionMode::DemoLocal,
                    Some(seed.clinician_email),
                    None,
                    seed.patients,
                    DesktopCitationCatalog {
                        by_workspace: seed.citations_by_workspace,
                    },
                )
            }
        }
    }
}

/// Runs the initial sign-in and workspace-open bootstrap on a Bevy `App` hosting `CurieClinicClientPlugin`.
pub fn bootstrap_clinic_session(
    app: &mut bevy::prelude::App,
    clinician_email: Option<String>,
    explicit_token: Option<String>,
) {
    app.world_mut()
        .write_message(ClinicActionMessage(ClinicAction::BootTab));
    app.update();

    let Some(email) = clinician_email else {
        app.update();
        return;
    };

    let token = if let Some(tok) = explicit_token {
        tok
    } else {
        app.world_mut()
            .write_message(ClinicActionMessage(ClinicAction::SubmitSignInEmail {
                email: email.clone(),
            }));
        app.update();
        match &app.world().resource::<ClinicSessionShell>().sign_in_form {
            SignInFormStatus::LinkRequested {
                proof_token: Some(tok),
                ..
            } => tok.clone(),
            _ => String::new(),
        }
    };

    if !token.is_empty() {
        app.world_mut()
            .write_message(ClinicActionMessage(ClinicAction::VerifySignInToken {
                email,
                token,
                entry: SignInEntryMethod::DirectForm,
            }));
        app.update();
    }

    // Sync server roster into DesktopWorkspaceState if running against live `--server`.
    let roster_patients: Vec<DesktopPatientEntry> =
        match &app.world().resource::<TodayRosterView>().state {
            PatientRosterState::Loaded { patients, .. } => patients
                .iter()
                .map(|p| DesktopPatientEntry {
                    workspace_id: p.patient_workspace_id.clone(),
                    display_name: p.display_name.clone(),
                    summary_line: format!("Workspace {}", p.patient_workspace_id.as_str()),
                })
                .collect(),
            _ => Vec::new(),
        };

    let first_ws_id = {
        let mut ws_state = app.world_mut().resource_mut::<DesktopWorkspaceState>();
        if ws_state.patients.is_empty() && !roster_patients.is_empty() {
            ws_state.patients = roster_patients;
        }
        ws_state
            .patients
            .first()
            .map(|entry| entry.workspace_id.clone())
    };

    if let Some(ws_id) = first_ws_id {
        app.world_mut()
            .write_message(ClinicActionMessage(ClinicAction::OpenPatientWorkspace {
                workspace_id: ws_id.clone(),
                chat_id: None,
            }));
        app.update();

        // Populate initial citations & greeting transcript entry for active workspace.
        let cards = app
            .world()
            .resource::<DesktopCitationCatalog>()
            .cards_for(&ws_id);
        let mut ws_state = app.world_mut().resource_mut::<DesktopWorkspaceState>();
        if !cards.is_empty() {
            ws_state.citations = cards;
            ws_state.selected_citation_index = 0;
        }
        if ws_state.transcript.is_empty()
            && let Some(active) = ws_state.active_patient().cloned()
        {
            ws_state.transcript.push(DesktopTranscriptEntry {
                speaker: "Curie Summary".to_owned(),
                body: format!("{} - {}", active.display_name, active.summary_line),
            });
        }
    }

    // Flush deferred AccessKit entity spawns into EfUiCatalogPlugin's A11yProjection.
    app.update();
}

/// Helper verifying live transport reachability for integration tests without mutating UI state.
pub fn fetch_live_whoami(
    transport: &mut dyn ClinicTransport,
    session: Option<&ef_core::SessionId>,
    now_secs: u64,
) -> Result<ef_clinic_domain::auth::WhoAmIPayload, ef_clinic_client::TransportError> {
    transport.whoami(session, now_secs)
}
