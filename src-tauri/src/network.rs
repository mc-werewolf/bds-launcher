use futures_util::{SinkExt, StreamExt};
use igd_next::{aio::tokio::search_gateway, PortMappingProtocol, SearchOptions};
use serde::{Deserialize, Serialize};
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::{
    collections::HashMap,
    fs,
    net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket},
    path::Path,
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{net::UdpSocket as TokioUdpSocket, sync::mpsc};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, http::HeaderValue, Message},
};

const BDS_PORT: u16 = 19132;
const RELAY_FIRST_PORT: u16 = 20000;
const RELAY_LAST_PORT: u16 = 20099;
const RELAY_PREFERENCE_FILE: &str = "network-relay-port.json";
const DIRECTORY_URL: &str = "https://mc-werewolf.com/api/network/v1/servers";
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

#[derive(Clone, Default)]
pub struct NetworkState(Arc<Mutex<Option<Session>>>);

#[derive(Clone)]
struct Session {
    id: String,
    token: String,
    endpoint: Option<Endpoint>,
}

#[derive(Clone)]
struct Endpoint {
    host_name: String,
    host_port: u16,
    mode: &'static str,
}

#[derive(Deserialize)]
struct RegistrationResponse {
    id: String,
    token: String,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct RelayPortPreference {
    port: u16,
    #[serde(default)]
    client_id: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishResult {
    pub server_id: String,
    pub public_address: Option<String>,
    pub local_address: Option<String>,
    pub port: u16,
    pub firewall_requested: bool,
    pub upnp_mapped: bool,
    pub warning: Option<String>,
}

pub async fn publish(state: NetworkState, install_root: &Path) -> Result<PublishResult, String> {
    if use_central_relay_publish() {
        let relay_preference = relay_preference(install_root);
        let mut endpoint = None;
        let mut warning = Some("Waiting for the central relay assignment.".to_owned());
        let session = register_with_preference(
            None,
            Some(relay_preference.port),
            Some(&relay_preference.client_id),
        )
        .await?;
        heartbeat(&session).await?;
        *state
            .0
            .lock()
            .map_err(|_| "繝阪ャ繝医Ρ繝ｼ繧ｯ迥ｶ諷九ｒ菫晏ｭ倥〒縺阪∪縺帙ｓ縺ｧ縺励◆")? =
            Some(session.clone());
        spawn_heartbeat(state.clone());
        spawn_relay(
            state.clone(),
            session.clone(),
            install_root.to_path_buf(),
            Some(relay_preference.client_id.clone()),
        );
        for _ in 0..80 {
            tokio::time::sleep(Duration::from_millis(250)).await;
            let assigned = state
                .0
                .lock()
                .ok()
                .and_then(|value| value.as_ref().and_then(|value| value.endpoint.clone()));
            if let Some(assigned) = assigned {
                let _ = save_relay_preference(
                    install_root,
                    &RelayPortPreference {
                        port: assigned.host_port,
                        client_id: relay_preference.client_id.clone(),
                    },
                );
                endpoint = Some(assigned);
                warning = None;
                break;
            }
        }
        return Ok(PublishResult {
            server_id: session.id,
            public_address: endpoint
                .as_ref()
                .map(|value| format!("{}:{}", value.host_name, value.host_port)),
            local_address: Some(format!("127.0.0.1:{BDS_PORT}")),
            port: BDS_PORT,
            firewall_requested: false,
            upnp_mapped: false,
            warning,
        });
    }

    let firewall_requested = request_firewall_rule()?;
    let direct = discover_direct_endpoint().await;
    let (mut endpoint, local_address, upnp_mapped, mut warning) = match direct {
        Ok((endpoint, local_address)) if is_public_ip(endpoint.host_name.parse().unwrap()) => {
            (Some(endpoint), Some(local_address), true, None)
        }
        Ok((_endpoint, local_address)) => (
            None,
            Some(local_address),
            true,
            Some(
                "CGNATまたは二重ルーターを検出しました。中央中継の割当を待っています。".to_owned(),
            ),
        ),
        Err(error) => (
            None,
            None,
            false,
            Some(format!("{error} 中央中継の割当を待っています。")),
        ),
    };
    let session = register(endpoint.clone()).await?;
    heartbeat(&session).await?;
    *state
        .0
        .lock()
        .map_err(|_| "ネットワーク状態を保存できませんでした")? = Some(session.clone());
    spawn_heartbeat(state.clone());
    if endpoint.is_none() {
        spawn_relay(
            state.clone(),
            session.clone(),
            install_root.to_path_buf(),
            None,
        );
        for _ in 0..80 {
            tokio::time::sleep(Duration::from_millis(250)).await;
            let assigned = state
                .0
                .lock()
                .ok()
                .and_then(|value| value.as_ref().and_then(|value| value.endpoint.clone()));
            if assigned.is_some() {
                endpoint = assigned;
                warning = None;
                break;
            }
        }
    }
    Ok(PublishResult {
        server_id: session.id,
        public_address: endpoint
            .as_ref()
            .map(|value| format!("{}:{}", value.host_name, value.host_port)),
        local_address,
        port: BDS_PORT,
        firewall_requested,
        upnp_mapped,
        warning,
    })
}

fn use_central_relay_publish() -> bool {
    true
}

async fn discover_direct_endpoint() -> Result<(Endpoint, String), String> {
    let gateway = search_gateway(SearchOptions {
        timeout: Some(Duration::from_secs(10)),
        single_search_timeout: Some(Duration::from_secs(3)),
        ..Default::default()
    })
    .await
    .map_err(|error| format!("UPnP対応ルーターを検出できませんでした: {error}."))?;
    let local_ip = local_ip_for_gateway(gateway.addr)?;
    let local_address = SocketAddr::new(local_ip, BDS_PORT);
    gateway
        .add_port(
            PortMappingProtocol::UDP,
            BDS_PORT,
            local_address,
            0,
            "Werewolf Bedrock Dedicated Server",
        )
        .await
        .map_err(|error| format!("ルーターでUDP {BDS_PORT}を開放できませんでした: {error}"))?;
    let public_ip = gateway
        .get_external_ip()
        .await
        .map_err(|error| format!("ルーターの公開IPを取得できませんでした: {error}"))?;
    Ok((
        Endpoint {
            host_name: public_ip.to_string(),
            host_port: BDS_PORT,
            mode: "direct",
        },
        local_address.to_string(),
    ))
}

async fn register(endpoint: Option<Endpoint>) -> Result<Session, String> {
    register_with_preference(endpoint, None, None).await
}

async fn register_with_preference(
    endpoint: Option<Endpoint>,
    relay_port_preference: Option<u16>,
    relay_client_id: Option<&str>,
) -> Result<Session, String> {
    let client = reqwest::Client::new();
    let registration = client
        .post(DIRECTORY_URL)
        .json(&serde_json::json!({
            "displayName": "Werewolf Server",
            "worldName": "Werewolf",
            "maxPlayers": 10,
            "relayPortPreference": relay_port_preference,
            "relayClientId": relay_client_id
        }))
        .send()
        .await
        .map_err(|error| format!("中央サーバーへ登録できませんでした: {error}"))?
        .error_for_status()
        .map_err(|error| format!("中央サーバーが登録を拒否しました: {error}"))?
        .json::<RegistrationResponse>()
        .await
        .map_err(|error| format!("中央サーバーの応答を解析できませんでした: {error}"))?;
    Ok(Session {
        id: registration.id,
        token: registration.token,
        endpoint,
    })
}

fn relay_preference(install_root: &Path) -> RelayPortPreference {
    let path = install_root.join(RELAY_PREFERENCE_FILE);
    let mut preference = RelayPortPreference {
        port: RELAY_FIRST_PORT + stable_relay_port_offset(install_root) as u16,
        client_id: new_relay_client_id(install_root),
    };
    if let Ok(content) = fs::read_to_string(&path) {
        if let Ok(value) = serde_json::from_str::<RelayPortPreference>(&content) {
            if (RELAY_FIRST_PORT..=RELAY_LAST_PORT).contains(&value.port) {
                preference.port = value.port;
            }
            if valid_relay_client_id(&value.client_id) {
                preference.client_id = value.client_id;
            }
        }
    }
    let _ = save_relay_preference(install_root, &preference);
    preference
}

fn save_relay_preference(
    install_root: &Path,
    preference: &RelayPortPreference,
) -> Result<(), String> {
    if !(RELAY_FIRST_PORT..=RELAY_LAST_PORT).contains(&preference.port) {
        return Ok(());
    }
    fs::create_dir_all(install_root)
        .map_err(|error| format!("failed to create network preference directory: {error}"))?;
    let content = serde_json::to_string_pretty(preference)
        .map_err(|error| format!("failed to serialize network preference: {error}"))?;
    fs::write(install_root.join(RELAY_PREFERENCE_FILE), content)
        .map_err(|error| format!("failed to save network preference: {error}"))
}

fn stable_relay_port_offset(install_root: &Path) -> u8 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in install_root.to_string_lossy().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    (hash % u64::from(RELAY_LAST_PORT - RELAY_FIRST_PORT + 1)) as u8
}

fn new_relay_client_id(install_root: &Path) -> String {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in install_root.to_string_lossy().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("launcher-{hash:016x}-{nanos:x}-{}", std::process::id())
}

fn valid_relay_client_id(value: &str) -> bool {
    if value.len() < 8 || value.len() > 128 {
        return false;
    }
    value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

async fn heartbeat(session: &Session) -> Result<(), String> {
    let (status, mode, host_name, host_port) = match &session.endpoint {
        Some(endpoint) => (
            "online",
            endpoint.mode,
            Some(endpoint.host_name.as_str()),
            Some(endpoint.host_port),
        ),
        None => ("starting", "pending", None, None),
    };
    reqwest::Client::new()
        .put(format!("{DIRECTORY_URL}/{}/heartbeat", session.id))
        .bearer_auth(&session.token)
        .json(&serde_json::json!({
            "playerCount": 0,
            "maxPlayers": 10,
            "status": status,
            "connectionMode": mode,
            "hostName": host_name,
            "hostPort": host_port
        }))
        .send()
        .await
        .map_err(|error| format!("heartbeatを送信できませんでした: {error}"))?
        .error_for_status()
        .map_err(|error| format!("heartbeatが拒否されました: {error}"))?;
    Ok(())
}

fn spawn_heartbeat(state: NetworkState) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(30)).await;
            let session = state.0.lock().ok().and_then(|value| value.clone());
            let Some(session) = session else { break };
            let _ = heartbeat(&session).await;
        }
    });
}

fn spawn_relay(
    state: NetworkState,
    session: Session,
    install_root: std::path::PathBuf,
    relay_client_id: Option<String>,
) {
    tauri::async_runtime::spawn(async move {
        loop {
            if relay_once(
                state.clone(),
                session.clone(),
                &install_root,
                relay_client_id.as_deref(),
            )
            .await
            .is_ok()
            {
                break;
            }
            tokio::time::sleep(Duration::from_secs(5)).await;
            let active = state
                .0
                .lock()
                .ok()
                .and_then(|value| value.as_ref().map(|value| value.id == session.id))
                .unwrap_or(false);
            if !active {
                break;
            }
        }
    });
}

async fn relay_once(
    state: NetworkState,
    session: Session,
    install_root: &Path,
    relay_client_id: Option<&str>,
) -> Result<(), String> {
    let url = format!(
        "wss://mc-werewolf.com/api/network/v1/servers/{}/relay",
        session.id
    );
    let mut request = url
        .into_client_request()
        .map_err(|error| format!("中継URLを作成できませんでした: {error}"))?;
    request.headers_mut().insert(
        "Authorization",
        HeaderValue::from_str(&format!("Bearer {}", session.token))
            .map_err(|error| format!("中継認証を作成できませんでした: {error}"))?,
    );
    let (websocket, _) = connect_async(request)
        .await
        .map_err(|error| format!("中央中継へ接続できませんでした: {error}"))?;
    let (mut writer, mut reader) = websocket.split();
    let (outbound, mut outbound_receiver) = mpsc::channel::<Vec<u8>>(256);
    let writer_task = tauri::async_runtime::spawn(async move {
        while let Some(frame) = outbound_receiver.recv().await {
            writer
                .send(Message::Binary(frame.into()))
                .await
                .map_err(|error| error.to_string())?;
        }
        Ok::<(), String>(())
    });
    let mut clients: HashMap<String, std::sync::Arc<TokioUdpSocket>> = HashMap::new();
    while let Some(message) = reader.next().await {
        match message.map_err(|error| format!("中継接続が切断されました: {error}"))? {
            Message::Text(text) => {
                #[derive(Deserialize)]
                struct Ready {
                    #[serde(rename = "type")]
                    kind: String,
                    #[serde(rename = "hostName")]
                    host_name: String,
                    port: u16,
                }
                let ready: Ready = serde_json::from_str(text.as_ref())
                    .map_err(|error| format!("中継準備通知を解析できませんでした: {error}"))?;
                if ready.kind == "ready" {
                    if let Some(client_id) = relay_client_id {
                        let _ = save_relay_preference(
                            install_root,
                            &RelayPortPreference {
                                port: ready.port,
                                client_id: client_id.to_owned(),
                            },
                        );
                    }
                    let endpoint = Endpoint {
                        host_name: ready.host_name,
                        host_port: ready.port,
                        mode: "relay",
                    };
                    let updated = {
                        let mut guard = state
                            .0
                            .lock()
                            .map_err(|_| "ネットワーク状態を更新できませんでした")?;
                        if let Some(current) = guard.as_mut() {
                            if current.id == session.id {
                                current.endpoint = Some(endpoint);
                                Some(current.clone())
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    };
                    if let Some(updated) = updated {
                        heartbeat(&updated).await?;
                    }
                }
            }
            Message::Binary(frame) => {
                let (remote_address, payload) = decode_relay_frame(&frame)?;
                let socket = if let Some(socket) = clients.get(&remote_address) {
                    socket.clone()
                } else {
                    let socket =
                        std::sync::Arc::new(TokioUdpSocket::bind("0.0.0.0:0").await.map_err(
                            |error| format!("ローカルUDPを作成できませんでした: {error}"),
                        )?);
                    socket
                        .connect((Ipv4Addr::LOCALHOST, BDS_PORT))
                        .await
                        .map_err(|error| format!("ローカルBDSへ接続できませんでした: {error}"))?;
                    let receive_socket = socket.clone();
                    let receive_address = remote_address.clone();
                    let receive_outbound = outbound.clone();
                    tauri::async_runtime::spawn(async move {
                        let mut buffer = vec![0_u8; max_datagram_size()];
                        while let Ok(size) = receive_socket.recv(&mut buffer).await {
                            if receive_outbound
                                .send(encode_relay_frame(&receive_address, &buffer[..size]))
                                .await
                                .is_err()
                            {
                                break;
                            }
                        }
                    });
                    clients.insert(remote_address.clone(), socket.clone());
                    socket
                };
                socket
                    .send(payload)
                    .await
                    .map_err(|error| format!("BDSへUDPを転送できませんでした: {error}"))?;
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
    drop(outbound);
    writer_task.await.map_err(|error| error.to_string())??;
    Err("中央中継が終了しました".to_owned())
}

fn max_datagram_size() -> usize {
    65_535
}

fn encode_relay_frame(address: &str, payload: &[u8]) -> Vec<u8> {
    let address = address.as_bytes();
    let mut frame = Vec::with_capacity(2 + address.len() + payload.len());
    frame.extend_from_slice(&(address.len() as u16).to_be_bytes());
    frame.extend_from_slice(address);
    frame.extend_from_slice(payload);
    frame
}

fn decode_relay_frame(frame: &[u8]) -> Result<(String, &[u8]), String> {
    if frame.len() < 2 {
        return Err("中継フレームが短すぎます".to_owned());
    }
    let address_length = u16::from_be_bytes([frame[0], frame[1]]) as usize;
    if address_length == 0 || frame.len() < 2 + address_length {
        return Err("中継フレームのアドレス長が不正です".to_owned());
    }
    let address = std::str::from_utf8(&frame[2..2 + address_length])
        .map_err(|error| format!("中継アドレスが不正です: {error}"))?;
    Ok((address.to_owned(), &frame[2 + address_length..]))
}

fn local_ip_for_gateway(gateway: SocketAddr) -> Result<IpAddr, String> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0))
        .map_err(|error| format!("ローカルネットワークを確認できませんでした: {error}"))?;
    socket
        .connect(gateway)
        .map_err(|error| format!("ルーターへ接続できませんでした: {error}"))?;
    socket
        .local_addr()
        .map(|address| address.ip())
        .map_err(|error| format!("ローカルIPを取得できませんでした: {error}"))
}

#[cfg(windows)]
fn request_firewall_rule() -> Result<bool, String> {
    let arguments = format!(
        "advfirewall firewall add rule name=\"Werewolf BDS UDP {BDS_PORT}\" dir=in action=allow protocol=UDP localport={BDS_PORT}"
    );
    let script = format!(
        "Start-Process -FilePath netsh.exe -Verb RunAs -WindowStyle Hidden -ArgumentList '{}' -Wait",
        arguments.replace('\'', "''")
    );
    let success = Command::new("powershell.exe")
        .args(["-NoProfile", "-WindowStyle", "Hidden", "-Command", &script])
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| format!("Firewall設定を開始できませんでした: {error}"))?
        .success();
    if success {
        Ok(true)
    } else {
        Err("Windows Firewall設定がキャンセルまたは失敗しました".to_owned())
    }
}

#[cfg(not(windows))]
fn request_firewall_rule() -> Result<bool, String> {
    Ok(false)
}

fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            !(ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_unspecified()
                || ip.octets()[0] == 100 && (64..=127).contains(&ip.octets()[1]))
        }
        IpAddr::V6(ip) => !(ip.is_loopback() || ip.is_unspecified() || ip.is_unique_local()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_public_and_non_public_addresses() {
        assert!(is_public_ip("8.8.8.8".parse().unwrap()));
        for address in ["127.0.0.1", "192.168.1.10", "10.0.0.1", "100.64.0.1"] {
            assert!(!is_public_ip(address.parse().unwrap()));
        }
    }

    #[test]
    fn relay_frame_round_trip() {
        let encoded = encode_relay_frame("203.0.113.10:54321", &[1, 2, 3]);
        let (address, payload) = decode_relay_frame(&encoded).unwrap();
        assert_eq!(address, "203.0.113.10:54321");
        assert_eq!(payload, &[1, 2, 3]);
    }

    #[test]
    fn reuses_saved_relay_port_preference() {
        let root = std::env::temp_dir().join(format!(
            "bds-launcher-relay-port-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();

        save_relay_preference(
            &root,
            &RelayPortPreference {
                port: 20042,
                client_id: "launcher-test-client".to_owned(),
            },
        )
        .unwrap();

        let preference = relay_preference(&root);
        assert_eq!(preference.port, 20042);
        assert_eq!(preference.client_id, "launcher-test-client");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn derives_relay_port_preference_inside_relay_range() {
        let root = std::env::temp_dir().join("bds-launcher-derived-relay-port-test");
        let preference = relay_preference(&root);

        assert!((RELAY_FIRST_PORT..=RELAY_LAST_PORT).contains(&preference.port));
        assert!(valid_relay_client_id(&preference.client_id));
    }
}
