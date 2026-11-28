// Run with node desktop/ui/src/verify/providerModels.test.mjs, then open the printed URL.
if (typeof document === "undefined") {
  const viteModule = "vite", urlModule = "node:url";
  const { createServer } = await import(/* @vite-ignore */ viteModule);
  const { fileURLToPath } = await import(/* @vite-ignore */ urlModule);
  const server = await createServer({
    root: fileURLToPath(new URL("../..", import.meta.url)),
    server: { port: 5198, strictPort: true },
    plugins: [{ name: "provider-test-page", configureServer(server) {
      server.middlewares.use("/__provider_result", (req, res) => {
        let result = "";
        req.on("data", (chunk) => { result += chunk; });
        req.on("end", () => {
          console.log(result);
          process.exitCode = result.startsWith("PASS ") ? 0 : 1;
          res.end("received");
          void server.close();
        });
      });
      server.middlewares.use("/__provider_test", async (_req, res) => {
        res.setHeader("Content-Type", "text/html");
        res.end(await server.transformIndexHtml("/__provider_test", '<div id="root"></div><pre id="result">Running</pre><script>addEventListener("error",e=>document.getElementById("result").textContent=e.message);addEventListener("unhandledrejection",e=>document.getElementById("result").textContent=String(e.reason))</script><script type="module" src="/src/verify/providerModels.test.mjs"></script>'));
      });
    } }],
  });
  await server.listen();
  console.log("Provider behavior checks: http://localhost:5198/__provider_test");
} else {
  const { createElement, useState } = await import("react");
  const { createRoot } = await import("react-dom/client");
  const { flushSync } = await import("react-dom");
  const { ProviderSettingsModal } = await import("../components/ProviderSettingsModal.tsx");
  const { tauriApi } = await import("../api/tauri.ts");
  const { usePiWorkspace } = await import("../state/piStore.ts");
  const checks = [];
  function assert(ok, message) { if (!ok) throw new Error(message); checks.push(message); }
  const tick = () => new Promise((resolve) => setTimeout(resolve, 20));
  const root = createRoot(document.getElementById("root"));
  let setOpen, actions, rerender;
  const provider = { id: "local", name: "Local", api: "openai-completions", base_url: "http://127.0.0.1:8000/v1", models: ["manual"], sign_in_url: null, has_api_key: true };
  tauriApi.listenPiEvents = async () => () => {};
  function App() {
    const [open, changeOpen] = useState(true);
    const [, setRevision] = useState(0);
    rerender = () => setRevision((revision) => revision + 1);
    setOpen = changeOpen;
    actions = usePiWorkspace().actions;
    return createElement(ProviderSettingsModal, { open, providers: [provider], onClose: () => changeOpen(false), onSave: actions.upsertProvider, onDelete: actions.deleteProvider });
  }
  function button(label) { return [...document.querySelectorAll("button")].find((el) => el.textContent.trim() === label); }
  async function click(label) { const el = button(label); if (!el || el.disabled) throw new Error(`Unavailable button: ${label}`); flushSync(() => el.click()); await tick(); }
  function input(label) { return [...document.querySelectorAll("label")].find((el) => el.textContent.includes(label))?.querySelector("input"); }
  async function type(el, value) {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set.call(el, value);
    flushSync(() => el.dispatchEvent(new Event("input", { bubbles: true })));
    await tick();
  }
  function deferred() { let resolve, reject; const promise = new Promise((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; }
  let pending, requests = [];
  let mutations = 0, bootstraps = 0;
  const bootstrap = tauriApi.bootstrapPiWorkspace;
  tauriApi.bootstrapPiWorkspace = async () => { bootstraps += 1; return bootstrap(); };
  tauriApi.upsertProvider = async () => { mutations += 1; throw new Error("disk write failed"); };
  tauriApi.deleteProvider = async () => { mutations += 1; throw new Error("delete failed"); };
  const previewScan = tauriApi.scanProviderModels;
  tauriApi.scanProviderModels = (draft) => { requests.push(draft); pending = deferred(); return pending.promise; };
  try {
    flushSync(() => root.render(createElement(App)));
    await tick();
    const initialBootstraps = bootstraps;
    await click("Edit");
    await click("Scan for models");
    assert(requests[0].provider_id === "local" && requests[0].api_key === undefined, "Saved-key scan uses the draft provider ID and omitted key");
    flushSync(rerender); await tick();
    pending.resolve(["discovered", "discovered", " ", "manual"]); await tick();
    assert(document.querySelector('[role="status"]').textContent.includes("Found 2"), "Candidates are normalized and deduplicated");
    assert(button("Add selected model"), "Parent rerender does not cancel an otherwise valid request");
    assert(document.body.textContent.includes("Models (1)"), "Scan leaves manual models unchanged until selection");
    await click("Add selected model");
    assert(document.body.textContent.includes("Models (2)") && button("Add selected model").disabled, "Explicit selection adds once and preserves manual model");
    await click("Add model"); await type(document.querySelector('[aria-label="New model ID"]'), "discovered"); await click("Save model");
    assert(document.body.textContent.includes("Models (2)"), "Manual entry cannot duplicate a discovered model");
    await click("Scan for models"); pending.reject(new Error("retryable scan failure")); await tick();
    assert(document.querySelector('[role="status"]').textContent.includes("retryable"), "Scan failure is visible and retry remains available");
    await click("Scan for models"); const stale = pending;
    await type(input("Base URL"), "http://127.0.0.1:9000/prefix/v1");
    stale.resolve(["stale-model"]); await tick();
    assert(!document.body.textContent.includes("stale-model") && button("Scan for models"), "Endpoint edits invalidate an in-flight result");
    await click("Scan for models"); const staleError = pending;
    await type(input("Display Name"), "New name"); staleError.reject(new Error("old request failure")); await tick();
    assert(!document.body.textContent.includes("old request failure"), "Input edits reject stale errors as well as stale catalogs");
    for (const [label, value] of [["Display Name", "Changed"], ["API Key", "draft-key"]]) {
      await click("Scan for models"); const old = pending;
      await type(input(label), value); old.resolve(["stale-input-model"]); await tick();
      assert(!document.body.textContent.includes("stale-input-model"), `${label} edits invalidate an in-flight result`);
    }
    await click("Scan for models"); const oldProtocol = pending;
    flushSync(() => document.querySelector('[aria-label="API Protocol"]').click()); await tick();
    flushSync(() => [...document.querySelectorAll('[role="option"]')].find((el) => el.textContent.includes("openai-responses")).click()); await tick();
    oldProtocol.resolve(["old-protocol-model"]); await tick();
    assert(!document.body.textContent.includes("old-protocol-model"), "Protocol edits invalidate an in-flight result");
    await click("Scan for models"); const reset = pending; await click("Reset form"); reset.resolve(["old-reset-model"]); await tick();
    assert(!document.body.textContent.includes("old-reset-model") && input("Provider ID").value === "", "Reset invalidates an in-flight result");
    await type(input("Base URL"), "http://localhost:8000");
    await click("Scan for models"); const unsaved = pending; await type(input("Provider ID"), "new-draft"); unsaved.resolve(["old-id-model"]); await tick();
    assert(requests.at(-1).provider_id === undefined && !document.body.textContent.includes("old-id-model"), "Unsaved provider scanning works and ID edits reject stale results");
    await click("Scan for models"); const closed = pending;
    await click("Close"); flushSync(() => setOpen(true)); await tick();
    closed.resolve(["closed-model"]); await tick();
    assert(!document.body.textContent.includes("closed-model"), "Closing and reopening rejects a prior result");
    assert(mutations === 0 && bootstraps === initialBootstraps, "Scanning never saves providers or restarts the workspace");
    await click("Edit");
    await click("Update provider");
    assert(document.querySelector('[role="alert"]')?.textContent.includes("disk write failed") && input("Provider ID").value === "local" && document.body.textContent.includes("Models (1)"), "Store save failure reaches the modal and preserves the draft");
    await click("Delete");
    await click("Confirm delete");
    assert(document.querySelector('[role="alert"]')?.textContent.includes("delete failed") && input("Provider ID").value === "local", "Store delete failure preserves edit state and confirmation");
    await type(input("API Key"), "temporary-key"); await click("Scan for models");
    assert(requests.at(-1).api_key === "temporary-key", "Entered key overrides only the scan request");
    pending.resolve([]); await tick();
    assert(document.querySelector('[role="status"]').textContent.includes("No models found"), "Empty catalog is visible and permits manual entry");
    await click("Scan for models"); const beforeClear = pending;
    const checkbox = document.querySelector('input[type="checkbox"]'); flushSync(() => checkbox.click()); await tick();
    beforeClear.resolve(["old-clear-model"]); await tick();
    assert(!document.body.textContent.includes("old-clear-model"), "Clear-key changes invalidate an in-flight result");
    await click("Scan for models");
    assert(requests.at(-1).api_key === "", "Clear key explicitly suppresses stored credentials during scan");
    pending.resolve([]); await tick();
    await previewScan({ api: "openai-completions", base_url: "http://localhost:8000" }).then(() => { throw new Error("Preview fabricated a catalog"); }, (error) => assert(error.message.includes("desktop app"), "Browser preview clearly rejects real-network scanning"));
    flushSync(() => document.querySelector('[aria-label="API Protocol"]').click()); await tick();
    flushSync(() => [...document.querySelectorAll('[role="option"]')].find((el) => el.textContent.includes("google-generative-ai")).click()); await tick();
    assert(button("Scan for models").disabled && document.querySelector('[role="status"]').textContent.includes("Google Generative AI") && document.body.textContent.includes("Models (1)"), "Google profiles retain manual models and clearly disable unsupported scanning");
    document.getElementById("result").textContent = `PASS ${checks.length} checks\n${checks.join("\n")}`;
  } catch (error) {
    document.getElementById("result").textContent = `FAIL after ${checks.length} checks: ${error.stack}`;
  }
  await fetch("/__provider_result", { method: "POST", body: document.getElementById("result").textContent });
}
