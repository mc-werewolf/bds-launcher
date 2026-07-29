use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{self, Cursor},
    path::{Path, PathBuf},
    process::Command,
};

const MAX_EXPANDED_SIZE: u64 = 1024 * 1024 * 1024;
const LOCAL_ADDON_VERSION: &str = "local";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LauncherConfig {
    registry_url: String,
    addons: Vec<LauncherAddon>,
}

#[derive(Debug, Deserialize)]
struct LauncherAddon {
    id: String,
    required: bool,
    #[serde(rename = "latestVersionUrl")]
    latest_version_url: String,
}

#[derive(Debug, Deserialize)]
struct VersionEnvelope {
    version: RegistryVersion,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RegistryVersion {
    version: String,
    file_size: u64,
    sha256: String,
    download_url: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateResult {
    addon_id: String,
    version: String,
    required: bool,
    updated: bool,
}

impl UpdateResult {
    pub fn addon_id(&self) -> &str {
        &self.addon_id
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct InstalledVersion {
    version: String,
    sha256: String,
}

pub async fn update_addons(
    config_url: &str,
    install_root: &Path,
    enabled_addons: &[String],
    auth_token: Option<&str>,
) -> Result<Vec<UpdateResult>, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|error| error.to_string())?;
    let config = client
        .get(config_url)
        .send()
        .await
        .map_err(|error| format!("ランチャー構成を取得できませんでした: {error}"))?
        .error_for_status()
        .map_err(|error| format!("ランチャー構成APIがエラーを返しました: {error}"))?
        .json::<LauncherConfig>()
        .await
        .map_err(|error| format!("ランチャー構成を解析できませんでした: {error}"))?;

    let addons_root = install_root.join("addons");
    fs::create_dir_all(&addons_root).map_err(|error| error.to_string())?;
    let mut results = Vec::with_capacity(config.addons.len());
    for addon in config
        .addons
        .into_iter()
        .filter(|addon| enabled_addons.contains(&addon.id))
    {
        validate_addon_id(&addon.id)?;
        let latest_url = absolute_url(&config.registry_url, &addon.latest_version_url);
        let addon_auth_token = private_addon_auth_token(&addon.id, auth_token);
        let release = with_optional_bearer(client.get(latest_url), addon_auth_token)
            .send()
            .await
            .map_err(|error| format!("{}の更新情報を取得できませんでした: {error}", addon.id))?
            .error_for_status()
            .map_err(|error| format!("{}の更新情報APIがエラーを返しました: {error}", addon.id))?
            .json::<VersionEnvelope>()
            .await
            .map_err(|error| format!("{}の更新情報を解析できませんでした: {error}", addon.id))?
            .version;
        let target = addons_root.join(&addon.id);
        if installed_version(&target)
            .as_ref()
            .is_some_and(|installed| {
                installed.version == release.version && installed.sha256 == release.sha256
            })
            && has_normalized_pack_layout(&target)
        {
            results.push(UpdateResult {
                addon_id: addon.id,
                version: release.version,
                required: addon.required,
                updated: false,
            });
            continue;
        }
        let download_url = absolute_url(&config.registry_url, &release.download_url);
        let bytes = with_optional_bearer(client.get(download_url), addon_auth_token)
            .send()
            .await
            .map_err(|error| format!("{}をダウンロードできませんでした: {error}", addon.id))?
            .error_for_status()
            .map_err(|error| format!("{}のダウンロードが拒否されました: {error}", addon.id))?
            .bytes()
            .await
            .map_err(|error| format!("{}を読み込めませんでした: {error}", addon.id))?;
        verify_archive(&bytes, &release)?;
        install_archive(&bytes, &target, &release)
            .map_err(|error| format!("{}をインストールできませんでした: {error}", addon.id))?;
        results.push(UpdateResult {
            addon_id: addon.id,
            version: release.version,
            required: addon.required,
            updated: true,
        });
    }
    if let Some(missing) = enabled_addons.iter().find(|id| {
        !results
            .iter()
            .any(|result| result.addon_id() == id.as_str())
    }) {
        return Err(format!("ランチャー構成に{missing}がありません"));
    }
    Ok(results)
}

pub fn install_local_addons(
    install_root: &Path,
    enabled_addons: &[String],
    packs_root: &Path,
    build_local_addons: bool,
) -> Result<Vec<UpdateResult>, String> {
    if !packs_root.is_dir() {
        return Err(format!(
            "Developer Mode packs root does not exist: {}",
            packs_root.display()
        ));
    }

    let addons_root = install_root.join("addons");
    fs::create_dir_all(&addons_root).map_err(|error| error.to_string())?;
    let mut results = Vec::with_capacity(enabled_addons.len());

    for addon_id in enabled_addons {
        validate_addon_id(addon_id)?;
        let source_name = local_addon_directory_name(addon_id)
            .ok_or_else(|| format!("Developer Mode does not know local source for {addon_id}"))?;
        let source = packs_root.join(source_name);
        if !source.is_dir() {
            return Err(format!(
                "Developer Mode source is missing for {addon_id}: {}",
                source.display()
            ));
        }

        if build_local_addons {
            run_local_build(&source).map_err(|error| format!("{addon_id}: {error}"))?;
        }

        let target = addons_root.join(addon_id);
        install_local_pack(&source, &target)
            .map_err(|error| format!("{addon_id} local pack install failed: {error}"))?;
        results.push(UpdateResult {
            addon_id: addon_id.to_owned(),
            version: LOCAL_ADDON_VERSION.to_owned(),
            required: true,
            updated: true,
        });
    }

    Ok(results)
}

fn local_addon_directory_name(addon_id: &str) -> Option<&'static str> {
    match addon_id {
        "kairo" => Some("kairo"),
        "kairo-database" => Some("kairo-database"),
        "werewolf-gamemanager" => Some("game-manager"),
        "werewolf-vanillapack" => Some("vanilla-pack"),
        "werewolf-bds-bridge" => Some("bds-bridge"),
        "werewolf-additionalroles-1" => Some("additional-roles-1"),
        "werewolf-additionalroles-4" => Some("additional-roles-4"),
        "werewolf-dev-tools" => Some("dev-tools"),
        _ => None,
    }
}

fn run_local_build(source: &Path) -> Result<(), String> {
    if !source.join("package.json").is_file() {
        return Ok(());
    }

    let executable = if cfg!(windows) { "pnpm.cmd" } else { "pnpm" };
    let output = Command::new(executable)
        .args(["run", "build:ci"])
        .current_dir(source)
        .output()
        .map_err(|error| {
            format!(
                "could not run pnpm build:ci in {}: {error}",
                source.display()
            )
        })?;

    if output.status.success() {
        return Ok(());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(format!(
        "pnpm build:ci failed in {}\n{}\n{}",
        source.display(),
        stdout.trim(),
        stderr.trim()
    ))
}

fn install_local_pack(source: &Path, target: &Path) -> io::Result<()> {
    let parent = target.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "install target has no parent")
    })?;
    let staging = parent.join(format!(
        ".{}.local-staging",
        target.file_name().unwrap_or_default().to_string_lossy()
    ));
    let backup = parent.join(format!(
        ".{}.local-backup",
        target.file_name().unwrap_or_default().to_string_lossy()
    ));
    remove_if_exists(&staging)?;
    remove_if_exists(&backup)?;
    fs::create_dir_all(&staging)?;

    let mut pack_count = 0;
    for directory_name in ["BP", "RP"] {
        let source_pack = source.join(directory_name);
        if source_pack.is_dir() {
            if !source_pack.join("manifest.json").is_file() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("{directory_name}/manifest.json is missing"),
                ));
            }
            copy_recursively(&source_pack, &staging.join(directory_name))?;
            pack_count += 1;
        }
    }
    if pack_count == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "local source contains neither a BP nor an RP pack",
        ));
    }

    fs::write(
        staging.join(".kairo-version.json"),
        serde_json::to_vec_pretty(&InstalledVersion {
            version: LOCAL_ADDON_VERSION.to_owned(),
            sha256: LOCAL_ADDON_VERSION.to_owned(),
        })?,
    )?;
    if target.exists() {
        fs::rename(target, &backup)?;
    }
    if let Err(error) = fs::rename(&staging, target) {
        if backup.exists() {
            let _ = fs::rename(&backup, target);
        }
        return Err(error);
    }
    remove_if_exists(&backup)
}

fn private_addon_auth_token<'a>(addon_id: &str, auth_token: Option<&'a str>) -> Option<&'a str> {
    match addon_id {
        "werewolf-dev-tools" => auth_token,
        _ => None,
    }
}

fn with_optional_bearer(
    request: reqwest::RequestBuilder,
    auth_token: Option<&str>,
) -> reqwest::RequestBuilder {
    match auth_token {
        Some(token) if !token.trim().is_empty() => request.bearer_auth(token.trim()),
        _ => request,
    }
}

fn absolute_url(registry_url: &str, value: &str) -> String {
    if value.starts_with("http://") || value.starts_with("https://") {
        value.to_owned()
    } else {
        format!(
            "{}{}",
            registry_url.trim_end_matches('/'),
            if value.starts_with('/') {
                value.to_owned()
            } else {
                format!("/{value}")
            }
        )
    }
}

fn validate_addon_id(id: &str) -> Result<(), String> {
    if id.is_empty()
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(format!("不正なアドオンIDです: {id}"));
    }
    Ok(())
}

fn installed_version(target: &Path) -> Option<InstalledVersion> {
    serde_json::from_slice(&fs::read(target.join(".kairo-version.json")).ok()?).ok()
}

fn has_normalized_pack_layout(target: &Path) -> bool {
    ["BP", "RP"]
        .iter()
        .any(|name| target.join(name).join("manifest.json").is_file())
}

fn verify_archive(bytes: &[u8], release: &RegistryVersion) -> Result<(), String> {
    if bytes.len() as u64 != release.file_size {
        return Err(format!(
            "ファイルサイズが一致しません (expected {}, got {})",
            release.file_size,
            bytes.len()
        ));
    }
    let actual = format!("{:x}", Sha256::digest(bytes));
    if !actual.eq_ignore_ascii_case(&release.sha256) {
        return Err("SHA-256が一致しません".to_owned());
    }
    Ok(())
}

fn install_archive(bytes: &[u8], target: &Path, release: &RegistryVersion) -> io::Result<()> {
    let parent = target.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "install target has no parent")
    })?;
    let staging = parent.join(format!(
        ".{}.staging",
        target.file_name().unwrap_or_default().to_string_lossy()
    ));
    let backup = parent.join(format!(
        ".{}.backup",
        target.file_name().unwrap_or_default().to_string_lossy()
    ));
    remove_if_exists(&staging)?;
    remove_if_exists(&backup)?;
    fs::create_dir_all(&staging)?;
    if let Err(error) = extract_zip(bytes, &staging) {
        let _ = remove_if_exists(&staging);
        return Err(error);
    }
    if let Err(error) = normalize_pack_layout(&staging) {
        let _ = remove_if_exists(&staging);
        return Err(error);
    }
    fs::write(
        staging.join(".kairo-version.json"),
        serde_json::to_vec_pretty(&InstalledVersion {
            version: release.version.clone(),
            sha256: release.sha256.clone(),
        })?,
    )?;
    if target.exists() {
        fs::rename(target, &backup)?;
    }
    if let Err(error) = fs::rename(&staging, target) {
        if backup.exists() {
            let _ = fs::rename(&backup, target);
        }
        return Err(error);
    }
    remove_if_exists(&backup)
}

fn normalize_pack_layout(staging: &Path) -> io::Result<()> {
    let mut pack_count = 0;
    for (directory_name, archive_suffix) in [("BP", "-bp.zip"), ("RP", "-rp.zip")] {
        let directory = staging.join(directory_name);
        if !directory.is_dir() {
            let archives = fs::read_dir(staging)?
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| {
                    path.is_file()
                        && path
                            .file_name()
                            .and_then(|name| name.to_str())
                            .is_some_and(|name| name.to_ascii_lowercase().ends_with(archive_suffix))
                })
                .collect::<Vec<_>>();
            match archives.as_slice() {
                [] => {}
                [archive] => {
                    fs::create_dir_all(&directory)?;
                    let bytes = fs::read(archive)?;
                    extract_zip(&bytes, &directory)?;
                    fs::remove_file(archive)?;
                }
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("multiple nested {directory_name} archives found"),
                    ));
                }
            }
        }
        if directory.is_dir() {
            if !directory.join("manifest.json").is_file() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("{directory_name}/manifest.json is missing"),
                ));
            }
            pack_count += 1;
        }
    }
    if pack_count == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "archive contains neither a BP nor an RP pack",
        ));
    }
    Ok(())
}

fn extract_zip(bytes: &[u8], destination: &Path) -> io::Result<()> {
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
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "ZIP symlinks are not allowed",
            ));
        }
        expanded = expanded
            .checked_add(entry.size())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "ZIP size overflow"))?;
        if expanded > MAX_EXPANDED_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "ZIP expands beyond 1 GiB",
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

fn remove_if_exists(path: &PathBuf) -> io::Result<()> {
    if path.exists() {
        fs::remove_dir_all(path)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn zip_bytes(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut archive = zip::ZipWriter::new(Cursor::new(&mut bytes));
            for (name, content) in entries {
                archive
                    .start_file(*name, zip::write::SimpleFileOptions::default())
                    .unwrap();
                archive.write_all(content).unwrap();
            }
            archive.finish().unwrap();
        }
        bytes
    }

    #[test]
    fn resolves_relative_registry_urls() {
        assert_eq!(
            absolute_url("https://kairojs.com/", "/api/v1/addons/kairo"),
            "https://kairojs.com/api/v1/addons/kairo"
        );
    }

    #[test]
    fn rejects_unsafe_addon_ids() {
        assert!(validate_addon_id("game-manager").is_ok());
        assert!(validate_addon_id("../escape").is_err());
    }

    #[test]
    fn normalizes_nested_github_release_packs() {
        let root = std::env::temp_dir().join(format!(
            "bds-launcher-nested-pack-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let manifest = br#"{"header":{"uuid":"pack-id","version":[1,0,0]}}"#;
        fs::write(
            root.join("werewolf-test-v1.0.0-BP.zip"),
            zip_bytes(&[("manifest.json", manifest)]),
        )
        .unwrap();
        fs::write(
            root.join("werewolf-test-v1.0.0-RP.zip"),
            zip_bytes(&[("manifest.json", manifest)]),
        )
        .unwrap();

        normalize_pack_layout(&root).unwrap();

        assert!(root.join("BP/manifest.json").is_file());
        assert!(root.join("RP/manifest.json").is_file());
        assert!(!root.join("werewolf-test-v1.0.0-BP.zip").exists());
        assert!(!root.join("werewolf-test-v1.0.0-RP.zip").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn rejects_archives_without_a_pack() {
        let root = std::env::temp_dir().join(format!(
            "bds-launcher-empty-pack-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("readme.txt"), "not a pack").unwrap();

        let error = normalize_pack_layout(&root).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn detects_cache_that_still_contains_nested_archives() {
        let root = std::env::temp_dir().join(format!(
            "bds-launcher-cache-layout-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("werewolf-test-v1.0.0-BP.zip"), "nested").unwrap();
        assert!(!has_normalized_pack_layout(&root));

        fs::create_dir_all(root.join("BP")).unwrap();
        fs::write(root.join("BP/manifest.json"), "{}").unwrap();
        assert!(has_normalized_pack_layout(&root));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn installs_local_addon_from_workspace_pack_layout() {
        let root = std::env::temp_dir().join(format!(
            "bds-launcher-local-addon-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        let packs_root = root.join("packs");
        let game_manager = packs_root.join("game-manager");
        fs::create_dir_all(game_manager.join("BP")).unwrap();
        fs::create_dir_all(game_manager.join("RP")).unwrap();
        let manifest = br#"{"header":{"uuid":"pack-id","version":[1,0,0]}}"#;
        fs::write(game_manager.join("BP/manifest.json"), manifest).unwrap();
        fs::write(game_manager.join("RP/manifest.json"), manifest).unwrap();

        let install_root = root.join("install");
        let results = install_local_addons(
            &install_root,
            &["werewolf-gamemanager".to_owned()],
            &packs_root,
            false,
        )
        .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].addon_id, "werewolf-gamemanager");
        assert_eq!(results[0].version, LOCAL_ADDON_VERSION);
        assert!(install_root
            .join("addons/werewolf-gamemanager/BP/manifest.json")
            .is_file());
        assert!(install_root
            .join("addons/werewolf-gamemanager/RP/manifest.json")
            .is_file());
        let _ = fs::remove_dir_all(root);
    }
}
