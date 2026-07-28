const { invoke } = window.__TAURI__.core;
const EULA_AGREEMENT_KEY = "mc-werewolf:eula-agreed";
const APP_UPDATE_INTERVAL_MS = 15 * 60 * 1000;

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
  const serverScreen = document.querySelector("#server-screen");
  const selectWerewolfButton = document.querySelector("#select-werewolf-btn");
  const addonsBackButton = document.querySelector("#addons-back-btn");
  const confirmAddonsButton = document.querySelector("#confirm-addons-btn");
  const changeAddonsButton = document.querySelector("#change-addons-btn");
  const optionalAddonInputs = [...document.querySelectorAll(".optional-addon")];
  const privateAddonOptions = [...document.querySelectorAll("[data-private-addon]")];
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
  let lastConsoleOutput = "";

  const showOnly = (screen) => {
    [
      onboardingScreen,
      loadingScreen,
      launcherHomeScreen,
      addonSelectionScreen,
      serverScreen,
    ].forEach((element) => {
      element.hidden = element !== screen;
    });
  };

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
      const result = await invoke("prepare_server", { selectedAddons });
      const updated = result.addons.filter((addon) => addon.updated).length;
      serverDetails.textContent = [
        `World: ${result.bds.worldName}`,
        `BDS ${result.bds.version}`,
        `Add-ons: ${result.addons.length}（${updated}件更新）`,
        `Behavior Packs: ${result.bds.behaviorPacks}`,
        `Resource Packs: ${result.bds.resourcePacks}`,
      ].join("\n");
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

    await prepareServer();
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
