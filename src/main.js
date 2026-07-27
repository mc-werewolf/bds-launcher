const { invoke } = window.__TAURI__.core;
const EULA_AGREEMENT_KEY = "mc-werewolf:eula-agreed";

window.addEventListener("DOMContentLoaded", () => {
  const appUpdateEl = document.querySelector("#app-update");
  const appUpdateTitleEl = document.querySelector("#app-update-title");
  const appUpdateMessageEl = document.querySelector("#app-update-message");
  const appUpdateButton = document.querySelector("#app-update-btn");
  const onboardingScreen = document.querySelector("#onboarding-screen");
  const loadingScreen = document.querySelector("#loading-screen");
  const homeScreen = document.querySelector("#home-screen");
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
  let lastConsoleOutput = "";

  const showOnly = (screen) => {
    [appUpdateEl, onboardingScreen, loadingScreen, homeScreen].forEach((element) => {
      element.hidden = element !== screen;
    });
  };

  const checkAppUpdate = async () => {
    try {
      const update = await invoke("check_app_update");
      if (!update) return false;

      appUpdateTitleEl.textContent = `ランチャー ${update.version} を利用できます`;
      appUpdateMessageEl.textContent =
        `現在のバージョン: ${update.currentVersion}` +
        (update.notes ? `\n${update.notes}` : "");
      showOnly(appUpdateEl);
      return true;
    } catch (error) {
      console.warn("ランチャーの更新確認に失敗しました", error);
      return false;
    }
  };

  const prepareServer = async () => {
    showOnly(loadingScreen);
    loadingSpinner.hidden = false;
    loadingTitle.textContent = "サーバーを準備しています";
    loadingMessage.textContent =
      "BDS、ワールド、アドオンの最新版を確認しています。\n初回は数分かかる場合があります。";
    retryButton.hidden = true;

    try {
      const result = await invoke("prepare_server");
      const updated = result.addons.filter((addon) => addon.updated).length;
      serverDetails.textContent = [
        `World: ${result.bds.worldName}`,
        `BDS ${result.bds.version}`,
        `Add-ons: ${result.addons.length}（${updated}件更新）`,
        `Behavior Packs: ${result.bds.behaviorPacks}`,
        `Resource Packs: ${result.bds.resourcePacks}`,
      ].join("\n");
      showOnly(homeScreen);
      startConsolePolling();
    } catch (error) {
      loadingSpinner.hidden = true;
      loadingTitle.textContent = "準備できませんでした";
      loadingMessage.textContent = String(error);
      retryButton.hidden = false;
    }
  };

  appUpdateButton.addEventListener("click", async () => {
    appUpdateButton.disabled = true;
    appUpdateButton.textContent = "更新をダウンロードしています…";
    appUpdateMessageEl.textContent = "完了後、ランチャーを再起動します。";
    try {
      await invoke("install_app_update");
    } catch (error) {
      appUpdateMessageEl.textContent = String(error);
      appUpdateButton.textContent = "再試行";
      appUpdateButton.disabled = false;
    }
  });

  agreement.addEventListener("change", () => {
    onboardingButton.disabled = !agreement.checked;
  });

  onboardingButton.addEventListener("click", () => {
    localStorage.setItem(EULA_AGREEMENT_KEY, "true");
    void prepareServer();
  });

  retryButton.addEventListener("click", () => {
    void prepareServer();
  });

  const setRunningControls = (running) => {
    stopServerButton.hidden = !running;
    restartServerButton.hidden = !running;
    stopServerButton.disabled = false;
    restartServerButton.disabled = false;
    consoleCommand.disabled = !running;
    consoleSend.disabled = !running;
    consoleLive.textContent = running ? "LIVE" : "OFFLINE";
    consoleLive.classList.toggle("is-live", running);
    serverState.textContent = running ? "稼働中" : "停止中";
    serverState.classList.toggle("is-running", running);
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
    loadingMessage.textContent = "利用可能な更新を確認しています。";

    if (await checkAppUpdate()) return;

    if (localStorage.getItem(EULA_AGREEMENT_KEY) === "true") {
      await prepareServer();
      startConsolePolling();
    } else {
      showOnly(onboardingScreen);
    }
  };

  void start();
});
