#![forbid(unsafe_code)]

use ef_ai::Router;
use ef_core::{UserProfile, new_entity_id};
use ef_ml::EMBEDDING_DIM;
use ef_oplog::EntityType;
use ef_prompts::STRUCTURE_PROMPT_V1;
use ef_protocol::FeatureId;
use ef_sidecar::SidecarSupervisor;
use ef_storage::Database;
use ef_win32::resolve_app_paths;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let paths = resolve_app_paths();
    let profile = UserProfile::default();
    let router = Router;
    let supervisor = SidecarSupervisor::new();
    let _prompt = router
        .prompt_version(FeatureId::BraindumpStructure)
        .unwrap_or(STRUCTURE_PROMPT_V1);
    let _store_ok = Database::open_in_memory().is_ok();
    let _id = new_entity_id();

    tracing::info!(
        packaged = paths.is_packaged,
        local_dir = %paths.local_data_dir.display(),
        cluster_id = %profile.cluster_id(),
        embedding_dim = EMBEDDING_DIM,
        root_entity = EntityType::InboxItem.as_str(),
        sidecar_state = ?supervisor.state(),
        "Vun Phase 00 foundation initialized"
    );
}
