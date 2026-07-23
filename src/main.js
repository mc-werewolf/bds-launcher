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
  const serverStatusMessage = document.querySelector("#server-status-msg");
  const onboardingButton = document.querySelector("#onboarding-btn");
  const agreement = document.querySelector("#eula-agreement");
  let serverLaunch = null;

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

  startServerButton.addEventListener("click", async () => {
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
  });

  const start = async () => {
    showOnly(loadingScreen);
    loadingTitle.textContent = "ランチャーを確認しています";
    loadingMessage.textContent = "利用可能な更新を確認しています。";

    if (await checkAppUpdate()) return;

    if (localStorage.getItem(EULA_AGREEMENT_KEY) === "true") {
      await prepareServer();
    } else {
      showOnly(onboardingScreen);
    }
  };

  void start();
});
