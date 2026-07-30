mod bds;
mod network;
mod updater;

use std::collections::HashMap;
use tauri::Manager;
use tauri_plugin_updater::UpdaterExt;

const LAUNCHER_CONFIG_URL: &str = "https://mc-werewolf.com/api/launcher/v1/config";
const PRIVATE_ADDON_TOKEN_ENV: &str = "KAIRO_PRIVATE_ADDON_TOKEN";
const PENDING_UPDATE_DIR: &str = "pending-update";
const PENDING_UPDATE_PACKAGE: &str = "package.bin";
const PENDING_UPDATE_METADATA: &str = "metadata.json";
const REQUIRED_WEREWOLF_ADDONS: &[&str] = &[
    "kairo",
    "kairo-database",
    "werewolf-gamemanager",
    "werewolf-vanillapack",
    "werewolf-bds-bridge",
];
const OPTIONAL_WEREWOLF_ADDONS: &[&str] = &[
    "werewolf-additionalroles-1",
    "werewolf-additionalroles-4",
    "werewolf-dev-tools",
    "werewolf-replay",
];

#[derive(Clone, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AppUpdate {
    current_version: String,
    version: String,
    notes: Option<String>,
    #[serde(default)]
    downloaded: bool,
}

#[derive(Default)]
struct AppUpdateState(tokio::sync::Mutex<()>);

#[tauri::command]
fn pending_app_update(app: tauri::AppHandle) -> Result<Option<AppUpdate>, String> {
    let current_version = app.package_info().version.to_string();
    let update_root = pending_update_root(&app)?;
    let metadata_path = update_root.join(PENDING_UPDATE_METADATA);
    let package_path = update_root.join(PENDING_UPDATE_PACKAGE);
    let Some(metadata) = std::fs::read(&metadata_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<AppUpdate>(&bytes).ok())
    else {
        return Ok(None);
    };
    if metadata.version == current_version || !package_path.is_file() {
        let _ = std::fs::remove_dir_all(update_root);
        return Ok(None);
    }
    Ok(Some(AppUpdate {
        current_version,
        downloaded: true,
        ..metadata
    }))
}

#[tauri::command]
async fn check_app_update(app: tauri::AppHandle) -> Result<Option<AppUpdate>, String> {
    if let Some(pending) = pending_app_update(app.clone())? {
        return Ok(Some(pending));
    }
    let current_version = app.package_info().version.to_string();
    let update = app
        .updater()
        .map_err(|error| format!("更新機能を初期化できませんでした: {error}"))?
        .check()
        .await
        .map_err(|error| format!("更新情報を確認できませんでした: {error}"))?;
    Ok(update.map(|update| AppUpdate {
        current_version,
        version: update.version.to_string(),
        notes: update.body,
        downloaded: false,
    }))
}

#[tauri::command]
async fn download_app_update(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppUpdateState>,
) -> Result<Option<AppUpdate>, String> {
    let _guard = state.0.lock().await;
    if let Some(pending) = pending_app_update(app.clone())? {
        return Ok(Some(pending));
    }

    let current_version = app.package_info().version.to_string();
    let update = app
        .updater()
        .map_err(|error| format!("更新機能を初期化できませんでした: {error}"))?
        .check()
        .await
        .map_err(|error| format!("更新情報を確認できませんでした: {error}"))?;

    let Some(update) = update else {
        return Ok(None);
    };
    let metadata = AppUpdate {
        current_version,
        version: update.version.to_string(),
        notes: update.body.clone(),
        downloaded: true,
    };
    let package = update
        .download(|_, _| {}, || {})
        .await
        .map_err(|error| format!("更新をダウンロードできませんでした: {error}"))?;
    let update_root = pending_update_root(&app)?;
    std::fs::create_dir_all(&update_root)
        .map_err(|error| format!("更新保存先を作成できませんでした: {error}"))?;
    let package_path = update_root.join(PENDING_UPDATE_PACKAGE);
    let package_staging = update_root.join("package.tmp");
    std::fs::write(&package_staging, package)
        .map_err(|error| format!("更新を保存できませんでした: {error}"))?;
    if package_path.exists() {
        std::fs::remove_file(&package_path)
            .map_err(|error| format!("古い更新を削除できませんでした: {error}"))?;
    }
    std::fs::rename(package_staging, package_path)
        .map_err(|error| format!("更新を確定できませんでした: {error}"))?;
    std::fs::write(
        update_root.join(PENDING_UPDATE_METADATA),
        serde_json::to_vec_pretty(&metadata).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("更新情報を保存できませんでした: {error}"))?;
    Ok(Some(metadata))
}

#[tauri::command]
async fn install_app_update(
    app: tauri::AppHandle,
    process: tauri::State<'_, bds::ServerProcess>,
) -> Result<(), String> {
    let install_root = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("アプリデータディレクトリを取得できませんでした: {error}"))?;
    if bds::console_snapshot(&install_root, &process)?.running {
        return Err("サーバー稼働中です。停止後に更新を適用します。".to_owned());
    }
    let pending = pending_app_update(app.clone())?
        .ok_or_else(|| "適用待ちの更新はありません。".to_owned())?;
    let update = app
        .updater()
        .map_err(|error| format!("更新機能を初期化できませんでした: {error}"))?
        .check()
        .await
        .map_err(|error| format!("更新情報を確認できませんでした: {error}"))?
        .ok_or_else(|| "利用可能な更新はありません。".to_owned())?;
    if update.version.to_string() != pending.version {
        let _ = std::fs::remove_dir_all(pending_update_root(&app)?);
        return Err("ダウンロード済みの更新が最新版と一致しません。".to_owned());
    }
    let update_root = pending_update_root(&app)?;
    let package = std::fs::read(update_root.join(PENDING_UPDATE_PACKAGE))
        .map_err(|error| format!("ダウンロード済み更新を読み込めませんでした: {error}"))?;
    update
        .install(package)
        .map_err(|error| format!("更新をインストールできませんでした: {error}"))?;
    let _ = std::fs::remove_dir_all(update_root);
    app.restart();
}

fn pending_update_root(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|path| path.join(PENDING_UPDATE_DIR))
        .map_err(|error| format!("アプリデータディレクトリを取得できませんでした: {error}"))
}

#[tauri::command]
async fn prepare_server(
    app: tauri::AppHandle,
    selected_addons: Vec<String>,
    settings: bds::BdsSettings,
) -> Result<PrepareResult, String> {
    let unknown = selected_addons
        .iter()
        .find(|id| !OPTIONAL_WEREWOLF_ADDONS.contains(&id.as_str()));
    if let Some(id) = unknown {
        return Err(format!("選択できないアドオンです: {id}"));
    }
    let mut enabled_addons = REQUIRED_WEREWOLF_ADDONS
        .iter()
        .map(|id| (*id).to_owned())
        .collect::<Vec<_>>();
    for id in selected_addons {
        if !enabled_addons.contains(&id) {
            enabled_addons.push(id);
        }
    }
    let install_root = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("アプリデータディレクトリを取得できませんでした: {error}"))?;
    let addons = if settings.developer_mode && !settings.developer_packs_root.trim().is_empty() {
        let packs_root = std::path::Path::new(settings.developer_packs_root.trim());
        let (local_addons, remote_addons): (Vec<_>, Vec<_>) = enabled_addons
            .iter()
            .cloned()
            .partition(|id| updater::local_addon_available(id, packs_root));
        let private_token = private_addon_token();
        if remote_addons.iter().any(|id| id == "werewolf-dev-tools" || id == "werewolf-replay") && private_token.is_none() {
            return Err("private add-on token is required".to_owned());
        }

        let mut results = Vec::new();
        if !remote_addons.is_empty() {
            results.extend(
                updater::update_addons(
                    LAUNCHER_CONFIG_URL,
                    &install_root,
                    &remote_addons,
                    private_token.as_deref(),
                )
                .await?,
            );
        }
        if !local_addons.is_empty() {
            results.extend(updater::install_local_addons(
                &install_root,
                &local_addons,
                packs_root,
                settings.developer_build_local_addons,
            )?);
        }
        order_addons(results, &enabled_addons)
    } else {
        let private_token = private_addon_token();
        if enabled_addons.iter().any(|id| id == "werewolf-dev-tools" || id == "werewolf-replay") && private_token.is_none() {
            return Err("private add-on token is required".to_owned());
        }
        updater::update_addons(
            LAUNCHER_CONFIG_URL,
            &install_root,
            &enabled_addons,
            private_token.as_deref(),
        )
        .await?
    };
    let addon_ids = addons
        .iter()
        .map(|result| result.addon_id().to_owned())
        .collect::<Vec<_>>();
    let bds = bds::prepare_bds(&install_root, &addon_ids, &settings).await?;
    Ok(PrepareResult { addons, bds })
}

fn order_addons(
    addons: Vec<updater::UpdateResult>,
    enabled_addons: &[String],
) -> Vec<updater::UpdateResult> {
    let mut by_id = addons
        .into_iter()
        .map(|addon| (addon.addon_id().to_owned(), addon))
        .collect::<HashMap<_, _>>();
    enabled_addons
        .iter()
        .filter_map(|id| by_id.remove(id))
        .collect()
}

#[derive(serde::Serialize)]
struct PrepareResult {
    addons: Vec<updater::UpdateResult>,
    bds: bds::BdsStatus,
}

#[tauri::command]
fn private_addons_enabled() -> bool {
    private_addon_token().is_some()
}

fn private_addon_token() -> Option<String> {
    std::env::var(PRIVATE_ADDON_TOKEN_ENV)
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

#[tauri::command]
fn start_server(
    app: tauri::AppHandle,
    process: tauri::State<'_, bds::ServerProcess>,
) -> Result<bds::LaunchResult, String> {
    let install_root = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("アプリデータディレクトリを取得できませんでした: {error}"))?;
    bds::start_bds(&install_root, &process)
}

#[tauri::command]
async fn publish_server(
    state: tauri::State<'_, network::NetworkState>,
) -> Result<network::PublishResult, String> {
    network::publish(state.inner().clone()).await
}

#[tauri::command]
fn stop_server(
    app: tauri::AppHandle,
    process: tauri::State<'_, bds::ServerProcess>,
) -> Result<(), String> {
    let install_root = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("アプリデータディレクトリを取得できませんでした: {error}"))?;
    bds::stop_bds(&install_root, &process, false)
}

#[tauri::command]
fn bridge_status(app: tauri::AppHandle) -> Result<bds::BridgeRuntimeStatus, String> {
    let install_root = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("アプリデータディレクトリを取得できませんでした: {error}"))?;
    Ok(bds::bridge_runtime_status(&install_root))
}

#[tauri::command]
fn server_console(
    app: tauri::AppHandle,
    process: tauri::State<'_, bds::ServerProcess>,
) -> Result<bds::ConsoleSnapshot, String> {
    let install_root = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("アプリデータディレクトリを取得できませんでした: {error}"))?;
    bds::console_snapshot(&install_root, &process)
}

#[tauri::command]
fn send_server_command(
    app: tauri::AppHandle,
    command: String,
    process: tauri::State<'_, bds::ServerProcess>,
) -> Result<(), String> {
    let install_root = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("アプリデータディレクトリを取得できませんでした: {error}"))?;
    bds::send_command(&install_root, &process, &command)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .manage(bds::ServerProcess::default())
        .manage(network::NetworkState::default())
        .manage(AppUpdateState::default())
        .invoke_handler(tauri::generate_handler![
            pending_app_update,
            check_app_update,
            download_app_update,
            install_app_update,
            prepare_server,
            start_server,
            publish_server,
            stop_server,
            bridge_status,
            server_console,
            send_server_command,
            private_addons_enabled
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
