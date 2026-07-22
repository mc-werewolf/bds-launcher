mod updater;

use tauri::Manager;

const LAUNCHER_CONFIG_URL: &str = "https://mc-werewolf.com/api/launcher/v1/config";

#[tauri::command]
fn start_server() -> String {
    "(TODO) ここでBDSサーバープロセスを起動する".to_string()
}

#[tauri::command]
async fn update_addons(app: tauri::AppHandle) -> Result<Vec<updater::UpdateResult>, String> {
    let install_root = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("アプリデータディレクトリを取得できませんでした: {error}"))?;
    updater::update_addons(LAUNCHER_CONFIG_URL, &install_root).await
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![update_addons, start_server])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
