use serde::{Deserialize, Serialize};
use sha2::Digest;
#[cfg(target_os = "windows")]
use std::os::windows::{io::AsRawHandle, process::CommandExt};
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{self, Cursor, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::Mutex,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
#[cfg(target_os = "windows")]
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE},
    System::{
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
            SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        },
        Threading::{
            OpenProcess, QueryFullProcessImageNameW, TerminateProcess,
            PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
        },
    },
};

const DOWNLOAD_LINKS_URL: &str =
    "https://net-secondary.web.minecraft-services.net/api/v1.0/download/links";
const WORLD_NAME: &str = "Werewolf";
const WORLD_METADATA_URL: &str = "https://mc-werewolf.com/api/world/latest";
const MAX_BDS_EXPANDED_SIZE: u64 = 2 * 1024 * 1024 * 1024;
const GRACEFUL_STOP_TIMEOUT: Duration = Duration::from_secs(30);
const SESSION_LOG_MARKER: &str = "[bds-launcher] BDS session starting";
const PROCESS_RECORD_FILE: &str = ".bds-launcher-process.json";
const CONSOLE_LOG_BYTES: u64 = 128 * 1024;
const LOG_ARCHIVE_DIR: &str = "logs";
const LOG_ARCHIVE_PREFIX: &str = "bedrock_server-";
const LOG_ARCHIVE_KEEP: usize = 10;
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

struct ManagedChild {
    child: Child,
    #[cfg(target_os = "windows")]
    _job: JobHandle,
}

impl std::ops::Deref for ManagedChild {
    type Target = Child;
    fn deref(&self) -> &Self::Target {
        &self.child
    }
}
impl std::ops::DerefMut for ManagedChild {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.child
    }
}

#[cfg(target_os = "windows")]
struct JobHandle(HANDLE);
#[cfg(target_os = "windows")]
unsafe impl Send for JobHandle {}
#[cfg(target_os = "windows")]
impl Drop for JobHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

pub struct ServerProcess(Mutex<Option<ManagedChild>>);

impl Default for ServerProcess {
    fn default() -> Self {
        Self(Mutex::new(None))
    }
}

impl Drop for ServerProcess {
    fn drop(&mut self) {
        let Ok(process) = self.0.get_mut() else {
            return;
        };
        if let Some(child) = process.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[derive(Debug, Deserialize)]
struct DownloadLinksEnvelope {
    result: DownloadLinks,
}
#[derive(Debug, Deserialize)]
struct DownloadLinks {
    links: Vec<DownloadLink>,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DownloadLink {
    download_type: String,
    download_url: String,
}
#[derive(Debug, Serialize, Deserialize)]
struct InstalledBds {
    download_url: String,
}
#[derive(Debug, Serialize, Deserialize)]
struct BdsProcessRecord {
    pid: u32,
    executable: String,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorldRelease {
    version: String,
    sha256: String,
    download_url: String,
}
#[derive(Debug, Deserialize)]
struct PackManifest {
    header: PackHeader,
    #[serde(default)]
    modules: Vec<PackModule>,
}
#[derive(Debug, Deserialize)]
struct PackHeader {
    uuid: String,
    #[serde(deserialize_with = "deserialize_pack_version")]
    version: Vec<u32>,
}
#[derive(Debug, Deserialize)]
struct PackModule {
    #[serde(rename = "type")]
    module_type: String,
    uuid: String,
}
#[derive(Debug, Clone, Serialize)]
struct WorldPack {
    pack_id: String,
    version: Vec<u32>,
}

fn deserialize_pack_version<'de, D>(deserializer: D) -> Result<Vec<u32>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum PackVersion {
        Components(Vec<u32>),
        Dotted(String),
    }

    match PackVersion::deserialize(deserializer)? {
        PackVersion::Components(version) => Ok(version),
        PackVersion::Dotted(version) => version
            .split('.')
            .map(|component| component.parse::<u32>().map_err(serde::de::Error::custom))
            .collect(),
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BdsStatus {
    pub version: String,
    pub updated: bool,
    pub world_name: String,
    pub behavior_packs: usize,
    pub resource_packs: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchResult {
    pub pid: u32,
    pub address: String,
    pub port: u16,
    pub world_name: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BridgeRuntimeStatus {
    pub state: String,
    pub message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConsoleSnapshot {
    pub output: String,
    pub running: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BdsSettings {
    pub server_name: String,
    pub server_port: u16,
    pub game_mode: String,
    pub difficulty: String,
    pub max_players: u32,
    pub online_mode: bool,
    pub allow_list: bool,
    pub allow_cheats: bool,
    pub view_distance: u32,
    pub tick_distance: u32,
    #[serde(default)]
    pub developer_mode: bool,
    #[serde(default = "default_developer_packs_root")]
    pub developer_packs_root: String,
    #[serde(default = "default_developer_build_local_addons")]
    pub developer_build_local_addons: bool,
}

impl Default for BdsSettings {
    fn default() -> Self {
        Self {
            server_name: "MC Werewolf Dev".to_owned(),
            server_port: 19132,
            game_mode: "survival".to_owned(),
            difficulty: "normal".to_owned(),
            max_players: 20,
            online_mode: true,
            allow_list: false,
            allow_cheats: true,
            view_distance: 10,
            tick_distance: 4,
            developer_mode: false,
            developer_packs_root: default_developer_packs_root(),
            developer_build_local_addons: default_developer_build_local_addons(),
        }
    }
}

fn default_developer_packs_root() -> String {
    String::new()
}

fn default_developer_build_local_addons() -> bool {
    true
}

impl BdsSettings {
    fn server_properties(&self) -> Vec<(&'static str, String)> {
        let defaults = Self::default();
        vec![
            (
                "server-name",
                sanitize_property_text(&self.server_name, &defaults.server_name, 64),
            ),
            ("level-name", WORLD_NAME.to_owned()),
            ("server-port", normalize_port(self.server_port).to_string()),
            (
                "gamemode",
                normalize_choice(
                    &self.game_mode,
                    &["survival", "creative", "adventure"],
                    &defaults.game_mode,
                ),
            ),
            (
                "difficulty",
                normalize_choice(
                    &self.difficulty,
                    &["peaceful", "easy", "normal", "hard"],
                    &defaults.difficulty,
                ),
            ),
            ("max-players", self.max_players.clamp(1, 100).to_string()),
            ("online-mode", self.online_mode.to_string()),
            ("allow-list", self.allow_list.to_string()),
            ("allow-cheats", self.allow_cheats.to_string()),
            ("view-distance", self.view_distance.clamp(5, 32).to_string()),
            ("tick-distance", self.tick_distance.clamp(4, 12).to_string()),
            ("content-log-console-output-enabled", "true".to_owned()),
            ("content-log-file-enabled", "true".to_owned()),
        ]
    }
}

pub async fn prepare_bds(
    install_root: &Path,
    addon_ids: &[String],
    settings: &BdsSettings,
) -> Result<BdsStatus, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .map_err(|error| error.to_string())?;
    let links = client
        .get(DOWNLOAD_LINKS_URL)
        .send()
        .await
        .map_err(|error| format!("BDSダウンロード情報を取得できませんでした: {error}"))?
        .error_for_status()
        .map_err(|error| format!("BDSダウンロードAPIがエラーを返しました: {error}"))?
        .json::<DownloadLinksEnvelope>()
        .await
        .map_err(|error| format!("BDSダウンロード情報を解析できませんでした: {error}"))?;
    let download_url = links
        .result
        .links
        .into_iter()
        .find(|link| link.download_type == "serverBedrockWindows")
        .map(|link| link.download_url)
        .ok_or_else(|| "Windows用BDSダウンロードが見つかりませんでした".to_owned())?;
    let bds_root = install_root.join("bds");
    fs::create_dir_all(&bds_root).map_err(|error| error.to_string())?;
    let current = bds_root.join("current");
    let installed = fs::read(current.join(".werewolf-bds.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<InstalledBds>(&bytes).ok());
    let updated = !current.join("bedrock_server.exe").is_file()
        || installed
            .as_ref()
            .is_none_or(|value| value.download_url != download_url);
    if updated {
        let bytes = client
            .get(&download_url)
            .send()
            .await
            .map_err(|error| format!("BDSをダウンロードできませんでした: {error}"))?
            .error_for_status()
            .map_err(|error| format!("BDSダウンロードが拒否されました: {error}"))?
            .bytes()
            .await
            .map_err(|error| format!("BDS ZIPを読み込めませんでした: {error}"))?;
        install_bds(&bytes, &current, &download_url)
            .map_err(|error| format!("BDSをインストールできませんでした: {error}"))?;
    }
    install_managed_world(&client, &current).await?;
    let (behavior_packs, resource_packs) = apply_addons(install_root, &current, addon_ids)
        .map_err(|error| format!("アドオンをBDSへ適用できませんでした: {error}"))?;
    configure_bds_bridge(&current)
        .map_err(|error| format!("BDS Bridgeを設定できませんでした: {error}"))?;
    ensure_server_properties(&current, settings).map_err(|error| error.to_string())?;
    clear_session_log(&current).map_err(|error| error.to_string())?;
    Ok(BdsStatus {
        version: version_from_url(&download_url),
        updated,
        world_name: WORLD_NAME.to_owned(),
        behavior_packs,
        resource_packs,
    })
}

async fn install_managed_world(client: &reqwest::Client, bds_root: &Path) -> Result<(), String> {
    let world_root = bds_root.join("worlds").join(WORLD_NAME);
    if world_root.join(".werewolf-world.json").is_file() {
        return Ok(());
    }

    let response = client
        .get(WORLD_METADATA_URL)
        .send()
        .await
        .map_err(|error| format!("Werewolfワールド情報を取得できませんでした: {error}"))?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(());
    }
    let release = response
        .error_for_status()
        .map_err(|error| format!("WerewolfワールドAPIがエラーを返しました: {error}"))?
        .json::<WorldRelease>()
        .await
        .map_err(|error| format!("Werewolfワールド情報を解析できませんでした: {error}"))?;
    let download_url = if release.download_url.starts_with("http") {
        release.download_url.clone()
    } else {
        format!("https://mc-werewolf.com{}", release.download_url)
    };
    let bytes = client
        .get(download_url)
        .send()
        .await
        .map_err(|error| format!("Werewolfワールドをダウンロードできませんでした: {error}"))?
        .error_for_status()
        .map_err(|error| format!("Werewolfワールドのダウンロードが拒否されました: {error}"))?
        .bytes()
        .await
        .map_err(|error| format!("Werewolfワールドを読み込めませんでした: {error}"))?;
    let digest = format!("{:x}", sha2::Sha256::digest(&bytes));
    if digest != release.sha256 {
        return Err("WerewolfワールドのSHA-256が一致しません".to_owned());
    }
    install_world_archive(&bytes, &world_root, &release)
        .map_err(|error| format!("Werewolfワールドをインストールできませんでした: {error}"))
}

fn install_world_archive(
    bytes: &[u8],
    world_root: &Path,
    release: &WorldRelease,
) -> io::Result<()> {
    let worlds_root = world_root
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "world target has no parent"))?;
    let staging = worlds_root.join(".Werewolf.staging");
    let backup = worlds_root.join(".Werewolf.backup");
    remove_dir(&staging)?;
    remove_dir(&backup)?;
    fs::create_dir_all(&staging)?;
    extract_zip(bytes, &staging, MAX_BDS_EXPANDED_SIZE)?;

    let content_root = find_level_root(&staging)?.ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "ワールドにlevel.datがありません",
        )
    })?;
    if content_root != staging {
        let normalized = worlds_root.join(".Werewolf.normalized");
        remove_dir(&normalized)?;
        fs::rename(&content_root, &normalized)?;
        remove_dir(&staging)?;
        fs::rename(&normalized, &staging)?;
    }
    fs::write(
        staging.join(".werewolf-world.json"),
        serde_json::to_vec_pretty(release)?,
    )?;
    if world_root.exists() {
        fs::rename(world_root, &backup)?;
    }
    if let Err(error) = fs::rename(&staging, world_root) {
        if backup.exists() {
            let _ = fs::rename(&backup, world_root);
        }
        return Err(error);
    }
    remove_dir(&backup)
}

fn find_level_root(root: &Path) -> io::Result<Option<PathBuf>> {
    if root.join("level.dat").is_file() {
        return Ok(Some(root.to_path_buf()));
    }
    for entry in fs::read_dir(root)? {
        let path = entry?.path();
        if path.is_dir() && path.join("level.dat").is_file() {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

fn install_bds(bytes: &[u8], current: &Path, download_url: &str) -> io::Result<()> {
    let parent = current
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "BDS target has no parent"))?;
    let staging = parent.join(".current.staging");
    let backup = parent.join(".current.backup");
    remove_dir(&staging)?;
    remove_dir(&backup)?;
    fs::create_dir_all(&staging)?;
    extract_zip(bytes, &staging, MAX_BDS_EXPANDED_SIZE)?;
    if !staging.join("bedrock_server.exe").is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "BDS ZIPにbedrock_server.exeがありません",
        ));
    }
    fs::write(
        staging.join(".werewolf-bds.json"),
        serde_json::to_vec_pretty(&InstalledBds {
            download_url: download_url.to_owned(),
        })?,
    )?;
    if current.exists() {
        preserve_path(current, &staging, "worlds")?;
        for file in [
            "server.properties",
            "allowlist.json",
            "permissions.json",
            "config",
        ] {
            preserve_path(current, &staging, file)?;
        }
        fs::rename(current, &backup)?;
    }
    if let Err(error) = fs::rename(&staging, current) {
        if backup.exists() {
            let _ = fs::rename(&backup, current);
        }
        return Err(error);
    }
    remove_dir(&backup)
}

fn preserve_path(current: &Path, staging: &Path, name: &str) -> io::Result<()> {
    let source = current.join(name);
    if !source.exists() {
        return Ok(());
    }
    let destination = staging.join(name);
    if destination.exists() {
        if destination.is_dir() {
            fs::remove_dir_all(&destination)?;
        } else {
            fs::remove_file(&destination)?;
        }
    }
    copy_recursively(&source, &destination)
}

fn apply_addons(
    install_root: &Path,
    bds_root: &Path,
    addon_ids: &[String],
) -> io::Result<(usize, usize)> {
    let mut behavior = Vec::new();
    let mut resources = Vec::new();
    for addon_id in addon_ids {
        let addon = install_root.join("addons").join(addon_id);
        let behavior_installed = install_pack(
            &addon.join("BP"),
            &bds_root.join("behavior_packs").join(addon_id),
            &mut behavior,
        )?;
        let resource_installed = install_pack(
            &addon.join("RP"),
            &bds_root.join("resource_packs").join(addon_id),
            &mut resources,
        )?;
        if !behavior_installed && !resource_installed {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("{addon_id}にBPまたはRPがありません"),
            ));
        }
    }
    let world = bds_root.join("worlds").join(WORLD_NAME);
    fs::create_dir_all(&world)?;
    write_json_atomic(&world.join("world_behavior_packs.json"), &behavior)?;
    write_json_atomic(&world.join("world_resource_packs.json"), &resources)?;
    Ok((behavior.len(), resources.len()))
}

fn install_pack(source: &Path, target: &Path, packs: &mut Vec<WorldPack>) -> io::Result<bool> {
    if !source.is_dir() {
        return Ok(false);
    }
    let manifest: PackManifest = serde_json::from_slice(&fs::read(source.join("manifest.json"))?)?;
    if manifest.header.version.len() != 3 || manifest.header.uuid.trim().is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "pack manifest header is invalid",
        ));
    }
    if target.exists() {
        fs::remove_dir_all(target)?;
    }
    copy_recursively(source, target)?;
    packs.push(WorldPack {
        pack_id: manifest.header.uuid,
        version: manifest.header.version,
    });
    Ok(true)
}

fn configure_bds_bridge(bds_root: &Path) -> io::Result<()> {
    let manifest_path = bds_root
        .join("behavior_packs")
        .join("werewolf-bds-bridge")
        .join("manifest.json");
    if !manifest_path.is_file() {
        return Ok(());
    }

    let manifest: PackManifest = serde_json::from_slice(&fs::read(manifest_path)?)?;
    let script_uuid = manifest
        .modules
        .iter()
        .find(|module| module.module_type == "script")
        .map(|module| module.uuid.trim())
        .filter(|uuid| !uuid.is_empty())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "BDS Bridge manifestにscript module UUIDがありません",
            )
        })?;
    let config_root = bds_root.join("config").join(script_uuid);
    fs::create_dir_all(&config_root)?;
    write_json_atomic(
        &config_root.join("permissions.json"),
        &serde_json::json!({
            "allowed_modules": [
                "@minecraft/server",
                "@minecraft/server-ui",
                "@minecraft/server-admin",
                "@minecraft/server-net"
            ]
        }),
    )?;
    write_json_atomic(
        &config_root.join("variables.json"),
        &serde_json::json!({
            "mcWerewolfApiUrl": "https://mc-werewolf.com"
        }),
    )
}

fn ensure_server_properties(bds_root: &Path, settings: &BdsSettings) -> io::Result<()> {
    let path = bds_root.join("server.properties");
    let content = fs::read_to_string(&path).unwrap_or_default();
    let properties = settings.server_properties();
    let mut found = HashSet::new();
    let mut output = String::new();
    for line in content.lines() {
        if let Some((key, _)) = line.split_once('=') {
            if let Some((managed_key, value)) = properties
                .iter()
                .find(|(managed_key, _)| *managed_key == key)
            {
                output.push_str(managed_key);
                output.push('=');
                output.push_str(value);
                output.push('\n');
                found.insert(*managed_key);
                continue;
            }
        }
        output.push_str(line);
        output.push('\n');
    }
    for (key, value) in properties {
        if found.contains(key) {
            continue;
        }
        output.push_str(key);
        output.push('=');
        output.push_str(&value);
        output.push('\n');
    }
    fs::write(path, output)
}

fn normalize_port(port: u16) -> u16 {
    if port == 0 {
        19132
    } else {
        port
    }
}

fn normalize_choice(value: &str, allowed: &[&str], default: &str) -> String {
    let normalized = value.trim().to_ascii_lowercase();
    if allowed.contains(&normalized.as_str()) {
        normalized
    } else {
        default.to_owned()
    }
}

fn sanitize_property_text(value: &str, default: &str, max_len: usize) -> String {
    let mut sanitized = value
        .chars()
        .filter(|character| !matches!(character, '\r' | '\n' | '='))
        .collect::<String>()
        .trim()
        .to_owned();
    if sanitized.is_empty() {
        sanitized = default.to_owned();
    }
    sanitized.chars().take(max_len).collect()
}

pub fn bridge_runtime_status(install_root: &Path) -> BridgeRuntimeStatus {
    let log_path = install_root
        .join("bds")
        .join("current")
        .join("bedrock_server.log");
    let Ok(content) = fs::read_to_string(log_path) else {
        return BridgeRuntimeStatus {
            state: "waiting".to_owned(),
            message: "Bridgeの起動を待っています。".to_owned(),
        };
    };

    for line in content.lines().rev() {
        if let Some(message) = line.split("[werewolf-bds-bridge] Connected to ").nth(1) {
            return BridgeRuntimeStatus {
                state: "connected".to_owned(),
                message: format!("Bridge接続済み: {}", message.trim()),
            };
        }
        if let Some(message) = line.split("[werewolf-bds-bridge] Disconnected: ").nth(1) {
            return BridgeRuntimeStatus {
                state: "disconnected".to_owned(),
                message: format!("Bridge未接続: {}", message.trim()),
            };
        }
        if line.contains(SESSION_LOG_MARKER) {
            break;
        }
    }

    BridgeRuntimeStatus {
        state: "waiting".to_owned(),
        message: "Bridgeを初期化しています。".to_owned(),
    }
}

pub fn start_bds(install_root: &Path, process: &ServerProcess) -> Result<LaunchResult, String> {
    let mut guard = process
        .0
        .lock()
        .map_err(|_| "BDSプロセス状態を取得できませんでした")?;
    if let Some(child) = guard.as_mut() {
        if child
            .try_wait()
            .map_err(|error| error.to_string())?
            .is_none()
        {
            return Err("BDSは既に起動しています".to_owned());
        }
        *guard = None;
        remove_process_record(install_root);
    }
    if orphaned_bds_running(install_root)? {
        return Err("以前のBDSが残っています。先にサーバーを停止してください。".to_owned());
    }
    let bds_root = install_root.join("bds").join("current");
    let executable = bds_root.join("bedrock_server.exe");
    if !executable.is_file() {
        return Err("BDSが準備されていません".to_owned());
    }
    rotate_session_log(&bds_root).map_err(|error| error.to_string())?;
    let mut log = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(bds_root.join("bedrock_server.log"))
        .map_err(|error| error.to_string())?;
    writeln!(log, "{SESSION_LOG_MARKER}").map_err(|error| error.to_string())?;
    log.flush().map_err(|error| error.to_string())?;
    let error_log = log.try_clone().map_err(|error| error.to_string())?;
    let mut command = Command::new(&executable);
    command
        .current_dir(&bds_root)
        .stdin(Stdio::piped())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(error_log));
    #[cfg(target_os = "windows")]
    command.creation_flags(CREATE_NO_WINDOW);
    let mut child = command
        .spawn()
        .map_err(|error| format!("BDSを起動できませんでした: {error}"))?;
    let pid = child.id();
    #[cfg(target_os = "windows")]
    let job = match assign_kill_on_close_job(&child) {
        Ok(job) => job,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!(
                "BDSをWindows Job Objectへ登録できませんでした: {error}"
            ));
        }
    };
    if let Err(error) = write_process_record(install_root, pid, &executable) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!("BDSプロセス情報を保存できませんでした: {error}"));
    }
    *guard = Some(ManagedChild {
        child,
        #[cfg(target_os = "windows")]
        _job: job,
    });
    Ok(LaunchResult {
        pid,
        address: "127.0.0.1".to_owned(),
        port: server_port(&bds_root.join("server.properties")),
        world_name: WORLD_NAME.to_owned(),
    })
}

fn rotate_session_log(bds_root: &Path) -> io::Result<()> {
    let log_path = bds_root.join("bedrock_server.log");
    if !log_path.is_file() || log_path.metadata()?.len() == 0 {
        return Ok(());
    }

    let archive_root = bds_root.join(LOG_ARCHIVE_DIR);
    fs::create_dir_all(&archive_root)?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let mut archive_path = archive_root.join(format!("{LOG_ARCHIVE_PREFIX}{timestamp}.log"));
    let mut suffix = 1;
    while archive_path.exists() {
        archive_path = archive_root.join(format!("{LOG_ARCHIVE_PREFIX}{timestamp}-{suffix}.log"));
        suffix += 1;
    }
    fs::rename(&log_path, archive_path)?;
    prune_log_archives(&archive_root)
}

fn clear_session_log(bds_root: &Path) -> io::Result<()> {
    OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(bds_root.join("bedrock_server.log"))?;
    Ok(())
}

fn prune_log_archives(archive_root: &Path) -> io::Result<()> {
    let mut archives = fs::read_dir(archive_root)?
        .filter_map(Result::ok)
        .filter(|entry| {
            entry.file_type().is_ok_and(|file_type| file_type.is_file())
                && entry.file_name().to_str().is_some_and(|name| {
                    name.starts_with(LOG_ARCHIVE_PREFIX) && name.ends_with(".log")
                })
        })
        .collect::<Vec<_>>();
    archives.sort_by_key(|entry| entry.file_name());
    let remove_count = archives.len().saturating_sub(LOG_ARCHIVE_KEEP);
    for entry in archives.into_iter().take(remove_count) {
        fs::remove_file(entry.path())?;
    }
    Ok(())
}

pub fn console_snapshot(
    install_root: &Path,
    process: &ServerProcess,
) -> Result<ConsoleSnapshot, String> {
    let running = {
        let mut guard = process
            .0
            .lock()
            .map_err(|_| "BDSプロセス状態を取得できませんでした")?;
        match guard.as_mut() {
            Some(child) => child
                .try_wait()
                .map_err(|error| error.to_string())?
                .is_none(),
            None => orphaned_bds_running(install_root)?,
        }
    };
    let log_path = install_root
        .join("bds")
        .join("current")
        .join("bedrock_server.log");
    let output = read_file_tail(&log_path, CONSOLE_LOG_BYTES)
        .unwrap_or_default()
        .rsplit_once(SESSION_LOG_MARKER)
        .map_or_else(String::new, |(_, current_session)| {
            current_session.trim_start_matches(['\r', '\n']).to_owned()
        });
    Ok(ConsoleSnapshot { output, running })
}

pub fn send_command(
    install_root: &Path,
    process: &ServerProcess,
    command: &str,
) -> Result<(), String> {
    let command = command.trim().trim_start_matches('/').trim();
    if command.is_empty() {
        return Err("コマンドを入力してください".to_owned());
    }
    if command.eq_ignore_ascii_case("stop") {
        return Err("停止には「サーバー停止」ボタンを使用してください".to_owned());
    }
    if command.contains(['\r', '\n']) {
        return Err("コマンドは1行で入力してください".to_owned());
    }

    let mut guard = process
        .0
        .lock()
        .map_err(|_| "BDSプロセス状態を取得できませんでした")?;
    let child = guard
        .as_mut()
        .ok_or_else(|| "BDSは起動していません".to_owned())?;
    if child
        .try_wait()
        .map_err(|error| error.to_string())?
        .is_some()
    {
        return Err("BDSは起動していません".to_owned());
    }
    let stdin = child
        .stdin
        .as_mut()
        .ok_or_else(|| "BDSの標準入力を利用できません".to_owned())?;
    let log_path = install_root
        .join("bds")
        .join("current")
        .join("bedrock_server.log");
    if let Ok(mut log) = OpenOptions::new().create(true).append(true).open(log_path) {
        let _ = writeln!(log, "[bds-launcher] > {command}");
    }
    writeln!(stdin, "{command}")
        .map_err(|error| format!("コマンドを送信できませんでした: {error}"))?;
    stdin
        .flush()
        .map_err(|error| format!("コマンドを送信できませんでした: {error}"))
}

fn read_file_tail(path: &Path, max_bytes: u64) -> io::Result<String> {
    let mut file = File::open(path)?;
    let length = file.metadata()?.len();
    file.seek(SeekFrom::Start(length.saturating_sub(max_bytes)))?;
    let mut bytes = Vec::with_capacity(length.min(max_bytes) as usize);
    file.read_to_end(&mut bytes)?;
    let output = String::from_utf8_lossy(&bytes);
    Ok(if length > max_bytes {
        output
            .split_once('\n')
            .map_or_else(String::new, |(_, remainder)| remainder.to_owned())
    } else {
        output.into_owned()
    })
}

/// Stops the running BDS process, if any. Sends the "stop" console command
/// over stdin so the server saves the world before exiting; if it hasn't
/// exited within `GRACEFUL_STOP_TIMEOUT`, falls back to killing it.
pub fn stop_bds(
    install_root: &Path,
    process: &ServerProcess,
    allow_not_running: bool,
) -> Result<(), String> {
    let mut guard = process
        .0
        .lock()
        .map_err(|_| "BDSプロセス状態を取得できませんでした")?;
    let Some(child) = guard.as_mut() else {
        if terminate_orphaned_bds(install_root)? {
            remove_process_record(install_root);
            return Ok(());
        }
        if allow_not_running {
            remove_process_record(install_root);
            return Ok(());
        }
        return Err("BDSは起動していません".to_owned());
    };
    if child
        .try_wait()
        .map_err(|error| error.to_string())?
        .is_some()
    {
        *guard = None;
        remove_process_record(install_root);
        return Err("BDSは起動していません".to_owned());
    }

    if let Some(stdin) = child.stdin.as_mut() {
        let _ = stdin.write_all(b"stop\n");
        let _ = stdin.flush();
    }

    let deadline = Instant::now() + GRACEFUL_STOP_TIMEOUT;
    loop {
        if child
            .try_wait()
            .map_err(|error| error.to_string())?
            .is_some()
        {
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            break;
        }
        thread::sleep(Duration::from_millis(200));
    }
    *guard = None;
    remove_process_record(install_root);
    Ok(())
}

fn process_record_path(root: &Path) -> PathBuf {
    root.join("bds").join(PROCESS_RECORD_FILE)
}
fn write_process_record(root: &Path, pid: u32, executable: &Path) -> io::Result<()> {
    write_json_atomic(
        &process_record_path(root),
        &BdsProcessRecord {
            pid,
            executable: executable.to_string_lossy().into_owned(),
        },
    )
}
fn read_process_record(root: &Path) -> Option<BdsProcessRecord> {
    serde_json::from_slice(&fs::read(process_record_path(root)).ok()?).ok()
}
fn remove_process_record(root: &Path) {
    let _ = fs::remove_file(process_record_path(root));
}
#[cfg(target_os = "windows")]
fn assign_kill_on_close_job(child: &Child) -> io::Result<JobHandle> {
    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job.is_null() {
            return Err(io::Error::last_os_error());
        }
        let mut information: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        information.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &information as *const _ as *const std::ffi::c_void,
            std::mem::size_of_val(&information) as u32,
        ) == 0
        {
            let error = io::Error::last_os_error();
            let _ = CloseHandle(job);
            return Err(error);
        }
        if AssignProcessToJobObject(job, child.as_raw_handle() as HANDLE) == 0 {
            let error = io::Error::last_os_error();
            let _ = CloseHandle(job);
            return Err(error);
        }
        Ok(JobHandle(job))
    }
}
fn same_process_path(left: &Path, right: &Path) -> bool {
    left.to_string_lossy()
        .eq_ignore_ascii_case(&right.to_string_lossy())
}
fn orphaned_bds_running(root: &Path) -> Result<bool, String> {
    let Some(record) = read_process_record(root) else {
        return Ok(false);
    };
    match process_executable(record.pid).map_err(|error| error.to_string())? {
        Some(path) if same_process_path(&path, Path::new(&record.executable)) => Ok(true),
        _ => {
            remove_process_record(root);
            Ok(false)
        }
    }
}
fn terminate_orphaned_bds(root: &Path) -> Result<bool, String> {
    let Some(record) = read_process_record(root) else {
        return Ok(false);
    };
    let Some(path) = process_executable(record.pid).map_err(|error| error.to_string())? else {
        remove_process_record(root);
        return Ok(false);
    };
    if !same_process_path(&path, Path::new(&record.executable)) {
        remove_process_record(root);
        return Ok(false);
    }
    terminate_process(record.pid)
        .map_err(|error| format!("残存BDSを終了できませんでした: {error}"))?;
    Ok(true)
}
#[cfg(target_os = "windows")]
fn process_executable(pid: u32) -> io::Result<Option<PathBuf>> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return Ok(None);
        }
        let mut buffer = vec![0_u16; 32768];
        let mut length = buffer.len() as u32;
        let result = QueryFullProcessImageNameW(handle, 0, buffer.as_mut_ptr(), &mut length);
        let _ = CloseHandle(handle);
        if result == 0 {
            return Err(io::Error::last_os_error());
        }
        buffer.truncate(length as usize);
        Ok(Some(PathBuf::from(String::from_utf16_lossy(&buffer))))
    }
}
#[cfg(not(target_os = "windows"))]
fn process_executable(_pid: u32) -> io::Result<Option<PathBuf>> {
    Ok(None)
}
#[cfg(target_os = "windows")]
fn terminate_process(pid: u32) -> io::Result<()> {
    unsafe {
        let handle = OpenProcess(PROCESS_TERMINATE, 0, pid);
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        let result = TerminateProcess(handle, 0);
        let _ = CloseHandle(handle);
        if result == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}
#[cfg(not(target_os = "windows"))]
fn terminate_process(_pid: u32) -> io::Result<()> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "Windows only"))
}

fn server_port(path: &Path) -> u16 {
    fs::read_to_string(path)
        .ok()
        .and_then(|content| {
            content.lines().find_map(|line| {
                line.strip_prefix("server-port=")?
                    .trim()
                    .parse::<u16>()
                    .ok()
            })
        })
        .unwrap_or(19132)
}
fn version_from_url(url: &str) -> String {
    url.rsplit('/')
        .next()
        .and_then(|name| name.strip_prefix("bedrock-server-"))
        .and_then(|name| name.strip_suffix(".zip"))
        .unwrap_or("unknown")
        .to_owned()
}
fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    let temporary = path.with_extension("json.tmp");
    let mut file = File::create(&temporary)?;
    file.write_all(&serde_json::to_vec_pretty(value)?)?;
    file.sync_all()?;
    if path.exists() {
        fs::remove_file(path)?;
    }
    fs::rename(temporary, path)
}
fn extract_zip(bytes: &[u8], destination: &Path, limit: u64) -> io::Result<()> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))?;
    let mut expanded = 0_u64;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let relative = entry
            .enclosed_name()
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "ZIP contains an unsafe path")
            })?
            .to_owned();
        expanded = expanded
            .checked_add(entry.size())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "ZIP size overflow"))?;
        if expanded > limit {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "ZIP is too large",
            ));
        }
        let output = destination.join(relative);
        if entry.is_dir() {
            fs::create_dir_all(output)?;
        } else {
            if let Some(parent) = output.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut file = File::create(output)?;
            io::copy(&mut entry, &mut file)?;
        }
    }
    Ok(())
}
fn copy_recursively(source: &Path, destination: &Path) -> io::Result<()> {
    if source.is_file() {
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(source, destination)?;
        return Ok(());
    }
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        copy_recursively(&entry.path(), &destination.join(entry.file_name()))?;
    }
    Ok(())
}
fn remove_dir(path: &PathBuf) -> io::Result<()> {
    if path.exists() {
        fs::remove_dir_all(path)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_array_and_dotted_pack_versions() {
        for (value, expected) in [
            (r#"[1, 1, 356]"#, vec![1, 1, 356]),
            (r#""2.0.39""#, vec![2, 0, 39]),
        ] {
            #[derive(Deserialize)]
            struct VersionFixture {
                #[serde(deserialize_with = "deserialize_pack_version")]
                version: Vec<u32>,
            }
            let fixture: VersionFixture =
                serde_json::from_str(&format!(r#"{{"version":{value}}}"#)).unwrap();
            assert_eq!(fixture.version, expected);
        }
    }

    #[test]
    fn reads_bds_version_from_download_url() {
        assert_eq!(
            version_from_url("https://example.test/bin-win/bedrock-server-1.26.33.2.zip"),
            "1.26.33.2"
        );
    }

    #[test]
    fn configures_world_and_writes_server_settings() {
        let root = std::env::temp_dir().join(format!(
            "bds-launcher-properties-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("server.properties"),
            "level-name=Bedrock level\nallow-list=true\nserver-port=19132\n",
        )
        .unwrap();

        let settings = BdsSettings {
            server_name: "Dev Werewolf".to_owned(),
            server_port: 19133,
            game_mode: "creative".to_owned(),
            difficulty: "easy".to_owned(),
            max_players: 12,
            online_mode: false,
            allow_list: false,
            allow_cheats: true,
            view_distance: 12,
            tick_distance: 6,
            developer_mode: false,
            developer_packs_root: String::new(),
            developer_build_local_addons: false,
        };

        ensure_server_properties(&root, &settings).unwrap();
        let configured = fs::read_to_string(root.join("server.properties")).unwrap();
        assert!(configured.contains("server-name=Dev Werewolf\n"));
        assert!(configured.contains("level-name=Werewolf\n"));
        assert!(configured.contains("allow-list=false\n"));
        assert!(configured.contains("allow-cheats=true\n"));
        assert!(configured.contains("server-port=19133\n"));
        assert!(configured.contains("gamemode=creative\n"));
        assert!(configured.contains("difficulty=easy\n"));
        assert!(configured.contains("max-players=12\n"));
        assert!(configured.contains("online-mode=false\n"));
        assert!(configured.contains("view-distance=12\n"));
        assert!(configured.contains("tick-distance=6\n"));
        assert!(configured.contains("content-log-console-output-enabled=true\n"));
        assert!(configured.contains("content-log-file-enabled=true\n"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rotates_previous_session_log_and_prunes_archives() {
        let root = std::env::temp_dir().join(format!(
            "bds-launcher-log-rotation-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join(LOG_ARCHIVE_DIR)).unwrap();
        fs::write(root.join("bedrock_server.log"), "old session\n").unwrap();
        for index in 0..LOG_ARCHIVE_KEEP {
            fs::write(
                root.join(LOG_ARCHIVE_DIR)
                    .join(format!("{LOG_ARCHIVE_PREFIX}{index}.log")),
                format!("archive {index}\n"),
            )
            .unwrap();
        }

        rotate_session_log(&root).unwrap();

        assert!(!root.join("bedrock_server.log").exists());
        let archives = fs::read_dir(root.join(LOG_ARCHIVE_DIR))
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().unwrap().is_file())
            .collect::<Vec<_>>();
        assert_eq!(archives.len(), LOG_ARCHIVE_KEEP);
        assert!(archives.iter().any(|entry| {
            fs::read_to_string(entry.path())
                .unwrap()
                .contains("old session")
        }));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn clears_session_log_without_archiving() {
        let root = std::env::temp_dir().join(format!(
            "bds-launcher-log-clear-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("bedrock_server.log"), "old session\n").unwrap();

        clear_session_log(&root).unwrap();

        assert_eq!(
            fs::read_to_string(root.join("bedrock_server.log")).unwrap(),
            ""
        );
        assert!(!root.join(LOG_ARCHIVE_DIR).exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn grants_server_net_only_to_bds_bridge_module() {
        let root = std::env::temp_dir().join(format!(
            "bds-launcher-bridge-config-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let pack = root.join("behavior_packs").join("werewolf-bds-bridge");
        fs::create_dir_all(&pack).unwrap();
        fs::write(
            pack.join("manifest.json"),
            r#"{
                "header": {"uuid": "pack-id", "version": [0, 1, 0]},
                "modules": [
                    {"type": "data", "uuid": "data-id"},
                    {"type": "script", "uuid": "script-id"}
                ]
            }"#,
        )
        .unwrap();

        configure_bds_bridge(&root).unwrap();

        let permissions =
            fs::read_to_string(root.join("config/script-id/permissions.json")).unwrap();
        let variables = fs::read_to_string(root.join("config/script-id/variables.json")).unwrap();
        assert!(permissions.contains("@minecraft/server-net"));
        assert!(variables.contains("https://mc-werewolf.com"));
        assert!(!root.join("config/default/permissions.json").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn job_object_kills_child_when_launcher_handle_closes() {
        let mut command = Command::new("cmd.exe");
        command
            .args(["/C", "ping 127.0.0.1 -n 30 >nul"])
            .creation_flags(CREATE_NO_WINDOW);
        let mut child = command.spawn().unwrap();
        let job = assign_kill_on_close_job(&child).unwrap();

        drop(job);

        let deadline = Instant::now() + Duration::from_secs(5);
        while child.try_wait().unwrap().is_none() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(50));
        }
        assert!(child.try_wait().unwrap().is_some());
    }
}
