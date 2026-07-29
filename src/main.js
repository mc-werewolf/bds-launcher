const { invoke } = window.__TAURI__.core;
const EULA_AGREEMENT_KEY = "mc-werewolf:eula-agreed";
const BDS_SETTINGS_KEY = "mc-werewolf:bds-settings";
const APP_UPDATE_INTERVAL_MS = 15 * 60 * 1000;
const DEFAULT_BDS_SETTINGS = {
  serverName: "MC Werewolf Dev",
  serverPort: 19132,
  gameMode: "survival",
  difficulty: "normal",
  maxPlayers: 20,
  onlineMode: true,
  allowList: false,
  allowCheats: true,
  viewDistance: 10,
  tickDistance: 4,
  developerMode: false,
  developerPacksRoot: "E:\\.projects\\minecraft\\kairo-js\\packs",
  developerBuildLocalAddons: true,
};

window.addEventListener("DOMContentLoaded", () => {
  const appUpdateEl = document.querySelector("#app-update");
  const appUpdateTitleEl = document.querySelector("#app-update-title");
  const appUpdateMessageEl = document.querySelector("#app-update-message");
  const appUpdateButton = document.querySelector("#app-update-btn");
  const appUpdateLaterButton = document.querySelector("#app-update-later-btn");
  const onboardingScreen = document.querySelector("#onboarding-screen");
  const loadingScreen = document.querySelector("#loading-screen");
  const launcherHomeScreen = document.querySelector("#launcher-home-screen");
  const addonSelectionScreen = document.querySelector("#addon-selection-screen");
  const serverSettingsScreen = document.querySelector("#server-settings-screen");
  const serverScreen = document.querySelector("#server-screen");
  const selectWerewolfButton = document.querySelector("#select-werewolf-btn");
  const addonsBackButton = document.querySelector("#addons-back-btn");
  const confirmAddonsButton = document.querySelector("#confirm-addons-btn");
  const settingsBackButton = document.querySelector("#settings-back-btn");
  const confirmSettingsButton = document.querySelector("#confirm-settings-btn");
  const changeAddonsButton = document.querySelector("#change-addons-btn");
  const optionalAddonInputs = [...document.querySelectorAll(".optional-addon")];
  const privateAddonOptions = [...document.querySelectorAll("[data-private-addon]")];
  const settingsInputs = {
    serverName: document.querySelector("#setting-server-name"),
    serverPort: document.querySelector("#setting-server-port"),
    gameMode: document.querySelector("#setting-game-mode"),
    difficulty: document.querySelector("#setting-difficulty"),
    maxPlayers: document.querySelector("#setting-max-players"),
    onlineMode: document.querySelector("#setting-online-mode"),
    allowList: document.querySelector("#setting-allow-list"),
    allowCheats: document.querySelector("#setting-allow-cheats"),
    viewDistance: document.querySelector("#setting-view-distance"),
    tickDistance: document.querySelector("#setting-tick-distance"),
    developerMode: document.querySelector("#setting-developer-mode"),
    developerPacksRoot: document.querySelector("#setting-developer-packs-root"),
    developerBuildLocalAddons: document.querySelector("#setting-developer-build-local-addons"),
  };
  const loadingSpinner = document.querySelector("#loading-spinner");
  const loadingTitle = document.querySelector("#loading-title");
  const loadingMessage = document.querySelector("#loading-message");
  const retryButton = document.querySelector("#retry-btn");
  const serverDetails = document.querySelector("#server-details");
  const startServerButton = document.querySelector("#start-server-btn");
  const stopServerButton = document.querySelector("#stop-server-btn");
  const restartServerButton = document.querySelector("#restart-server-btn");
  const serverStatusMessage = document.querySelector("#server-status-msg");
  const serverState = document.querySelector("#server-state");
  const consoleOutput = document.querySelector("#console-output");
  const consoleForm = document.querySelector("#console-form");
  const consoleCommand = document.querySelector("#console-command");
  const consoleSend = document.querySelector("#console-send");
  const consoleLive = document.querySelector("#console-live");
  const consoleError = document.querySelector("#console-error");
  const onboardingButton = document.querySelector("#onboarding-btn");
  const agreement = document.querySelector("#eula-agreement");
  let serverLaunch = null;
  let bridgeStatusTimer = null;
  let consoleTimer = null;
  let appUpdateTimer = null;
  let pendingAppUpdate = null;
  let dismissedUpdateVersion = null;
  let selectedAddons = optionalAddonInputs.filter((input) => input.checked).map((input) => input.value);
  let bdsSettings = loadBdsSettings();
  let lastConsoleOutput = "";

  const showOnly = (screen) => {
    [
      onboardingScreen,
      loadingScreen,
      launcherHomeScreen,
      addonSelectionScreen,
      serverSettingsScreen,
      serverScreen,
    ].forEach((element) => {
      element.hidden = element !== screen;
    });
  };

  function clampInteger(value, min, max, fallback) {
    const parsed = Number.parseInt(String(value), 10);
    if (!Number.isFinite(parsed)) return fallback;
    return Math.min(max, Math.max(min, parsed));
  }

  function choice(value, allowed, fallback) {
    return allowed.includes(value) ? value : fallback;
  }

  function normalizeBdsSettings(value) {
    return {
      serverName: String(value?.serverName ?? DEFAULT_BDS_SETTINGS.serverName).replace(/[\r\n=]/g, "").trim().slice(0, 64) || DEFAULT_BDS_SETTINGS.serverName,
      serverPort: clampInteger(value?.serverPort, 1, 65535, DEFAULT_BDS_SETTINGS.serverPort),
      gameMode: choice(value?.gameMode, ["survival", "creative", "adventure"], DEFAULT_BDS_SETTINGS.gameMode),
      difficulty: choice(value?.difficulty, ["peaceful", "easy", "normal", "hard"], DEFAULT_BDS_SETTINGS.difficulty),
      maxPlayers: clampInteger(value?.maxPlayers, 1, 100, DEFAULT_BDS_SETTINGS.maxPlayers),
      onlineMode: Boolean(value?.onlineMode ?? DEFAULT_BDS_SETTINGS.onlineMode),
      allowList: Boolean(value?.allowList ?? DEFAULT_BDS_SETTINGS.allowList),
      allowCheats: Boolean(value?.allowCheats ?? DEFAULT_BDS_SETTINGS.allowCheats),
      viewDistance: clampInteger(value?.viewDistance, 5, 32, DEFAULT_BDS_SETTINGS.viewDistance),
      tickDistance: clampInteger(value?.tickDistance, 4, 12, DEFAULT_BDS_SETTINGS.tickDistance),
      developerMode: Boolean(value?.developerMode ?? DEFAULT_BDS_SETTINGS.developerMode),
      developerPacksRoot: String(value?.developerPacksRoot ?? DEFAULT_BDS_SETTINGS.developerPacksRoot).trim() || DEFAULT_BDS_SETTINGS.developerPacksRoot,
      developerBuildLocalAddons: Boolean(value?.developerBuildLocalAddons ?? DEFAULT_BDS_SETTINGS.developerBuildLocalAddons),
    };
  }

  function loadBdsSettings() {
    try {
      return normalizeBdsSettings(JSON.parse(localStorage.getItem(BDS_SETTINGS_KEY) || "null"));
    } catch {
      return { ...DEFAULT_BDS_SETTINGS };
    }
  }

  function saveBdsSettings(settings) {
    bdsSettings = normalizeBdsSettings(settings);
    localStorage.setItem(BDS_SETTINGS_KEY, JSON.stringify(bdsSettings));
  }

  function renderBdsSettings() {
    const settings = normalizeBdsSettings(bdsSettings);
    settingsInputs.serverName.value = settings.serverName;
    settingsInputs.serverPort.value = settings.serverPort;
    settingsInputs.gameMode.value = settings.gameMode;
    settingsInputs.difficulty.value = settings.difficulty;
    settingsInputs.maxPlayers.value = settings.maxPlayers;
    settingsInputs.onlineMode.checked = settings.onlineMode;
    settingsInputs.allowList.checked = settings.allowList;
    settingsInputs.allowCheats.checked = settings.allowCheats;
    settingsInputs.viewDistance.value = settings.viewDistance;
    settingsInputs.tickDistance.value = settings.tickDistance;
    settingsInputs.developerMode.checked = settings.developerMode;
    settingsInputs.developerPacksRoot.value = settings.developerPacksRoot;
    settingsInputs.developerBuildLocalAddons.checked = settings.developerBuildLocalAddons;
  }

  function collectBdsSettings() {
    return normalizeBdsSettings({
      serverName: settingsInputs.serverName.value,
      serverPort: settingsInputs.serverPort.value,
      gameMode: settingsInputs.gameMode.value,
      difficulty: settingsInputs.difficulty.value,
      maxPlayers: settingsInputs.maxPlayers.value,
      onlineMode: settingsInputs.onlineMode.checked,
      allowList: settingsInputs.allowList.checked,
      allowCheats: settingsInputs.allowCheats.checked,
      viewDistance: settingsInputs.viewDistance.value,
      tickDistance: settingsInputs.tickDistance.value,
      developerMode: settingsInputs.developerMode.checked,
      developerPacksRoot: settingsInputs.developerPacksRoot.value,
      developerBuildLocalAddons: settingsInputs.developerBuildLocalAddons.checked,
    });
  }

  function renderPrepareResult(result) {
    const updated = result.addons.filter((addon) => addon.updated).length;
    serverDetails.textContent = [
      `World: ${result.bds.worldName}`,
      `BDS ${result.bds.version}`,
      `Add-ons: ${result.addons.length} (${updated} updated)`,
      `Behavior Packs: ${result.bds.behaviorPacks}`,
      `Resource Packs: ${result.bds.resourcePacks}`,
      ...(bdsSettings.developerMode ? ["Mode: Developer (local packs)"] : []),
    ].join("\n");
  }

  const renderAppUpdate = () => {
    if (!pendingAppUpdate || pendingAppUpdate.version === dismissedUpdateVersion) {
      appUpdateEl.hidden = true;
      return;
    }
    appUpdateEl.hidden = false;
    if (pendingAppUpdate.downloaded) {
      appUpdateTitleEl.textContent = `ランチャー ${pendingAppUpdate.version} をダウンロード済み`;
      appUpdateMessageEl.textContent = serverLaunch
        ? "サーバーはそのまま稼働します。停止後に自動で更新します。"
        : "サーバー停止中のため、更新を適用できます。";
      appUpdateButton.disabled = Boolean(serverLaunch);
      appUpdateButton.textContent = serverLaunch ? "停止後に自動更新" : "更新を適用";
      appUpdateLaterButton.hidden = true;
    } else {
      appUpdateTitleEl.textContent = `ランチャー ${pendingAppUpdate.version} を利用できます`;
      appUpdateMessageEl.textContent = "更新をダウンロードしますか？ 後から更新しても、そのまま利用できます。";
      appUpdateButton.disabled = false;
      appUpdateButton.textContent = "ダウンロード";
      appUpdateLaterButton.hidden = false;
    }
  };

  const checkAppUpdate = async () => {
    try {
      pendingAppUpdate = await invoke("check_app_update");
      renderAppUpdate();
    } catch (error) {
      console.warn("ランチャーの更新確認に失敗しました", error);
    }
  };

  const scheduleAppUpdateChecks = () => {
    void checkAppUpdate();
    if (appUpdateTimer !== null) return;
    appUpdateTimer = window.setInterval(checkAppUpdate, APP_UPDATE_INTERVAL_MS);
  };

  const downloadAppUpdate = async () => {
    appUpdateButton.disabled = true;
    appUpdateLaterButton.disabled = true;
    appUpdateButton.textContent = "ダウンロード中…";
    appUpdateMessageEl.textContent = "署名を確認し、安全な更新として保存しています。";
    pendingAppUpdate = await invoke("download_app_update");
    appUpdateLaterButton.disabled = false;
    renderAppUpdate();
  };

  const applyAppUpdate = async () => {
    appUpdateButton.disabled = true;
    appUpdateButton.textContent = "更新を適用しています…";
    appUpdateMessageEl.textContent = "ランチャーを再起動します。";
    await invoke("install_app_update");
  };

  const prepareServer = async () => {
    showOnly(loadingScreen);
    loadingSpinner.hidden = false;
    loadingTitle.textContent = "サーバーを準備しています";
    loadingMessage.textContent =
      "BDS、ワールド、アドオンの最新版を確認しています。\n初回は数分かかる場合があります。";
    retryButton.hidden = true;

    try {
      const result = await invoke("prepare_server", { selectedAddons, settings: bdsSettings });
      renderPrepareResult(result);
      clearConsoleView();
      showOnly(serverScreen);
      startConsolePolling();
      scheduleAppUpdateChecks();
    } catch (error) {
      loadingSpinner.hidden = true;
      loadingTitle.textContent = "準備できませんでした";
      loadingMessage.textContent = String(error);
      retryButton.hidden = false;
    }
  };

  appUpdateButton.addEventListener("click", async () => {
    try {
      if (pendingAppUpdate?.downloaded) {
        await applyAppUpdate();
      } else {
        await downloadAppUpdate();
      }
    } catch (error) {
      appUpdateMessageEl.textContent = String(error);
      appUpdateButton.disabled = false;
      appUpdateButton.textContent = "再試行";
      appUpdateLaterButton.disabled = false;
    }
  });

  appUpdateLaterButton.addEventListener("click", () => {
    dismissedUpdateVersion = pendingAppUpdate?.version ?? null;
    renderAppUpdate();
  });

  agreement.addEventListener("change", () => {
    onboardingButton.disabled = !agreement.checked;
  });

  onboardingButton.addEventListener("click", () => {
    localStorage.setItem(EULA_AGREEMENT_KEY, "true");
    showOnly(launcherHomeScreen);
    scheduleAppUpdateChecks();
  });

  retryButton.addEventListener("click", () => {
    void prepareServer();
  });

  selectWerewolfButton.addEventListener("click", () => {
    showOnly(addonSelectionScreen);
  });

  addonsBackButton.addEventListener("click", () => {
    showOnly(launcherHomeScreen);
  });

  confirmAddonsButton.addEventListener("click", () => {
    selectedAddons = optionalAddonInputs
      .filter((input) => input.checked)
      .map((input) => input.value);
    if (selectedAddons.includes("werewolf-dev-tools")) {
      bdsSettings.allowCheats = true;
    }
    renderBdsSettings();
    showOnly(serverSettingsScreen);
  });

  settingsBackButton.addEventListener("click", () => {
    showOnly(addonSelectionScreen);
  });

  confirmSettingsButton.addEventListener("click", () => {
    saveBdsSettings(collectBdsSettings());
    void prepareServer();
  });

  changeAddonsButton.addEventListener("click", () => {
    showOnly(addonSelectionScreen);
  });

  const setRunningControls = (running) => {
    stopServerButton.hidden = !running;
    restartServerButton.hidden = !running;
    stopServerButton.disabled = false;
    restartServerButton.disabled = false;
    changeAddonsButton.disabled = running;
    consoleCommand.disabled = !running;
    consoleSend.disabled = !running;
    consoleLive.textContent = running ? "LIVE" : "OFFLINE";
    consoleLive.classList.toggle("is-live", running);
    serverState.textContent = running ? "稼働中" : "停止中";
    serverState.classList.toggle("is-running", running);
    renderAppUpdate();
  };

  const refreshConsole = async () => {
    try {
      const snapshot = await invoke("server_console");
      setRunningControls(snapshot.running);
      if (!snapshot.running && serverLaunch) {
        serverLaunch = null;
        stopBridgeStatusPolling();
        startServerButton.disabled = false;
        startServerButton.textContent = "サーバー起動";
        serverStatusMessage.textContent = "BDSプロセスが終了しました。";
      }
      if (snapshot.output !== lastConsoleOutput) {
        const wasNearBottom =
          consoleOutput.scrollHeight - consoleOutput.scrollTop - consoleOutput.clientHeight < 56;
        lastConsoleOutput = snapshot.output;
        consoleOutput.textContent =
          snapshot.output || "サーバーを起動すると、ここにログが表示されます。";
        if (wasNearBottom) {
          consoleOutput.scrollTop = consoleOutput.scrollHeight;
        }
      }
    } catch (error) {
      consoleError.textContent = `ログを取得できませんでした: ${error}`;
    }
  };

  const clearConsoleView = () => {
    lastConsoleOutput = "";
    consoleOutput.textContent = "サーバーを起動すると、ここにログが表示されます。";
    consoleError.textContent = "";
  };

  const startConsolePolling = () => {
    if (consoleTimer !== null) return;
    void refreshConsole();
    consoleTimer = window.setInterval(refreshConsole, 750);
  };

  const stopBridgeStatusPolling = () => {
    if (bridgeStatusTimer !== null) {
      window.clearInterval(bridgeStatusTimer);
      bridgeStatusTimer = null;
    }
  };

  const startBridgeStatusPolling = () => {
    stopBridgeStatusPolling();
    const updateBridgeStatus = async () => {
      try {
        const bridge = await invoke("bridge_status");
        serverDetails.dataset.bridgeStatus = bridge.state;
        const baseDetails = serverDetails.textContent
          .split("\n")
          .filter((line) => !line.startsWith("Bridge:"))
          .join("\n");
        serverDetails.textContent = `${baseDetails}\nBridge: ${bridge.message}`;
      } catch (error) {
        console.warn("Bridgeの状態を取得できませんでした", error);
      }
    };
    void updateBridgeStatus();
    bridgeStatusTimer = window.setInterval(updateBridgeStatus, 3000);
  };

  const launchAndPublish = async () => {
    startServerButton.disabled = true;
    startServerButton.textContent = "サーバーを起動しています…";
    serverStatusMessage.textContent = "BDSを起動しています。";

    if (!serverLaunch) {
      try {
        if (bdsSettings.developerMode) {
          serverStatusMessage.textContent = "Developer Mode: syncing local add-ons...";
          const result = await invoke("prepare_server", { selectedAddons, settings: bdsSettings });
          renderPrepareResult(result);
          clearConsoleView();
        }
        serverLaunch = await invoke("start_server");
      } catch (error) {
        serverStatusMessage.textContent = `サーバーを起動できませんでした: ${error}`;
        startServerButton.textContent = "サーバー起動";
        startServerButton.disabled = false;
        return;
      }
    }

    setRunningControls(true);
    startConsolePolling();
    startBridgeStatusPolling();
    startServerButton.textContent = "インターネットへ公開しています…";
    serverStatusMessage.textContent =
      "BDSを起動しました。Firewallと接続経路を設定しています。Windowsの確認画面が表示された場合は許可してください。";

    try {
      const published = await invoke("publish_server");
      serverStatusMessage.textContent = published.warning
        ? `サーバーを公開しました（ID: ${published.serverId}）。${published.warning}`
        : `サーバーを公開しました: ${published.publicAddress}（LAN: ${published.localAddress}）`;
      startServerButton.textContent = "サーバー公開中";
    } catch (error) {
      serverStatusMessage.textContent =
        `BDSはローカルで起動中です（PID ${serverLaunch.pid} / ${serverLaunch.address}:${serverLaunch.port}）` +
        `\nインターネットへ公開できませんでした: ${error}`;
      startServerButton.textContent = "公開を再試行";
      startServerButton.disabled = false;
    }
  };

  const stopServer = async () => {
    stopServerButton.disabled = true;
    restartServerButton.disabled = true;
    serverStatusMessage.textContent = "BDSを停止しています（ワールドを保存しています）…";

    try {
      await invoke("stop_server");
    } catch (error) {
      serverStatusMessage.textContent = `サーバーを停止できませんでした: ${error}`;
      stopServerButton.disabled = false;
      restartServerButton.disabled = false;
      return false;
    }

    serverLaunch = null;
    stopBridgeStatusPolling();
    setRunningControls(false);
    startServerButton.hidden = false;
    startServerButton.disabled = false;
    startServerButton.textContent = "サーバー起動";
    serverStatusMessage.textContent = "BDSを停止しました。";
    if (pendingAppUpdate?.downloaded) {
      serverStatusMessage.textContent = "BDSを停止しました。ランチャーを更新しています…";
      try {
        await applyAppUpdate();
      } catch (error) {
        serverStatusMessage.textContent = `BDSを停止しました。更新を適用できませんでした: ${error}`;
        renderAppUpdate();
      }
    }
    return true;
  };

  consoleForm.addEventListener("submit", async (event) => {
    event.preventDefault();
    const command = consoleCommand.value.trim();
    if (!command) return;

    consoleSend.disabled = true;
    consoleError.textContent = "";
    try {
      await invoke("send_server_command", { command });
      consoleCommand.value = "";
      await refreshConsole();
      consoleCommand.focus();
    } catch (error) {
      consoleError.textContent = String(error);
    } finally {
      consoleSend.disabled = false;
    }
  });

  startServerButton.addEventListener("click", () => {
    void launchAndPublish();
  });

  stopServerButton.addEventListener("click", () => {
    void stopServer();
  });

  restartServerButton.addEventListener("click", async () => {
    stopServerButton.disabled = true;
    restartServerButton.disabled = true;
    restartServerButton.textContent = "再起動しています…";

    const stopped = await stopServer();
    restartServerButton.textContent = "再起動";
    if (!stopped) return;

    if (!bdsSettings.developerMode) {
      await prepareServer();
    }
    void launchAndPublish();
  });

  const start = async () => {
    showOnly(loadingScreen);
    loadingTitle.textContent = "ランチャーを確認しています";
    loadingMessage.textContent = "適用待ちの更新を確認しています。";

    try {
      const privateAddonsEnabled = await invoke("private_addons_enabled");
      privateAddonOptions.forEach((option) => {
        option.hidden = !privateAddonsEnabled;
      });

      pendingAppUpdate = await invoke("pending_app_update");
      if (pendingAppUpdate) {
        loadingTitle.textContent = "ランチャーを更新しています";
        loadingMessage.textContent = `${pendingAppUpdate.version} を適用しています。`;
        await invoke("install_app_update");
        return;
      }
    } catch (error) {
      console.warn("適用待ちの更新を処理できませんでした", error);
    }

    if (localStorage.getItem(EULA_AGREEMENT_KEY) === "true") {
      showOnly(launcherHomeScreen);
      scheduleAppUpdateChecks();
    } else {
      showOnly(onboardingScreen);
    }
  };

  void start();
});
