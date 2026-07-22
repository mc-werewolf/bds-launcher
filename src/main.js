const { invoke } = window.__TAURI__.core;

window.addEventListener("DOMContentLoaded", () => {
  const updateStatusEl = document.querySelector("#update-status");
  const updateDetailsEl = document.querySelector("#update-details");
  const actionsEl = document.querySelector("#actions");
  const startServerMsgEl = document.querySelector("#start-server-msg");
  document.querySelector("#start-server-btn").addEventListener("click", async () => {
    startServerMsgEl.textContent = await invoke("start_server");
  });

  invoke("update_addons")
    .then((results) => {
      const updated = results.filter((result) => result.updated).length;
      updateStatusEl.textContent = `準備完了（${updated}件更新）`;
      updateDetailsEl.textContent = results
        .map((result) => `${result.addonId} ${result.version}${result.updated ? " — 更新済み" : ""}`)
        .join("\n");
      actionsEl.hidden = false;
    })
    .catch((error) => {
      updateStatusEl.textContent = "更新に失敗しました";
      updateDetailsEl.textContent = String(error);
    });
});
