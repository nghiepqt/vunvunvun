#![forbid(unsafe_code)]

use ef_core::UserProfile;
use ef_win32::resolve_app_paths;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let paths = resolve_app_paths();
    let profile = UserProfile::default();

    tracing::info!(
        packaged = paths.is_packaged,
        local_dir = %paths.local_data_dir.display(),
        cluster_id = %profile.cluster_id(),
        "Vun Phase 00 foundation initialized"
    );
}
