<script lang="ts">
  import { onMount } from "svelte";
  import { cloudDevices, cloudEnableLocalApi, cloudLogin, cloudLogout, cloudStatus, reselectCt, type CloudDevice, type Region } from "../lib/api";
  import { app, errText, setting } from "../lib/state.svelte";
  import { t } from "../lib/i18n.svelte";

  let account = $state<{ email: string; region: Region; expired: boolean } | null>(null);
  let email = $state("");
  let region = $state<Region>("eu");
  let password = $state("");
  let devices = $state<CloudDevice[]>();
  let error = $state("");
  let loading = $state(false);
  let enabling = $state(false);
  let enableMsg = $state("");

  // Codes meter de l'API cloud (VNSD) ; les noms de produit ne se traduisent pas.
  const meters = ["CT001", "Shelly Pro 3EM", "P1", "CT002", "CT003", "Shelly EM Gen3", "Shelly Pro EM50", "EcoTracker", "SMR-P1", "SMR-IR", "SMR-TIC", "TPM2-100CT", "Stromleser"];
  const meter = $derived(app.settings.ctReselectMeter);
  // Seuls les CT Bluetooth (CT002=3, CT003=4) prennent un MAC ; les autres se joignent par IP (réglée dans l'app Marstek).
  const needsMac = $derived(meter === 3 || meter === 4);
  const isIpMeter = $derived(meter !== undefined && meter !== 0 && !needsMac);
  const macOk = $derived(!needsMac || /^[0-9a-fA-F]{12}$/.test(app.settings.ctReselectMac ?? ""));
  let reselecting = $state(false);
  let reselectMsg = $state("");

  const needsPassword = $derived(!account || account.expired);
  const label = (d: CloudDevice) => `${d.name ?? d.devid}${d.type ? ` (${d.type})` : ""}`;
  const chosen = $derived(app.settings.cloudDevice);

  onMount(async () => {
    try {
      account = await cloudStatus();
      if (account) ({ email, region } = account);
    } catch (e) {
      error = errText(e);
    }
  });

  /** Une seule lecture : liste des appareils, avec le seul appareil choisi d'office. */
  async function fetchDevices() {
    devices = await cloudDevices();
    if (!chosen && devices.length === 1) setting("cloudDevice", { id: devices[0].devid, name: label(devices[0]) });
  }

  async function submit(e: Event) {
    e.preventDefault();
    error = "";
    loading = true;
    try {
      if (needsPassword) {
        await cloudLogin(email.trim(), password, region);
        account = { email: email.trim(), region, expired: false };
      } else {
        devices = undefined;
      }
      password = "";
      await fetchDevices();
    } catch (e) {
      password = "";
      error = errText(e);
      if (account) account.expired = (await cloudStatus().catch(() => null))?.expired ?? false;
    } finally {
      loading = false;
    }
  }

  function pick(id: string) {
    const d = devices?.find((x) => x.devid === id);
    setting("cloudDevice", d ? { id, name: label(d) } : null);
  }

  /** Rouvre l'Open API locale de l'appareil choisi via le cloud (quand elle a été coupée après un redémarrage). */
  async function enableLocal() {
    if (!chosen) return;
    error = "";
    enableMsg = "";
    enabling = true;
    try {
      await cloudEnableLocalApi(chosen.id);
      enableMsg = t("account.enableSent");
    } catch (e) {
      error = errText(e);
    } finally {
      enabling = false;
    }
  }

  /** Lit la config CT réelle de la batterie et la renvoie ; affiche la réponse de l'appareil. */
  async function reselect() {
    if (!chosen) return;
    error = "";
    reselectMsg = "";
    reselecting = true;
    try {
      reselectMsg = await reselectCt(chosen.id, meter ?? 0, needsMac ? app.settings.ctReselectMac : undefined);
    } catch (e) {
      error = errText(e);
    } finally {
      reselecting = false;
    }
  }

  async function logout() {
    error = "";
    try {
      await cloudLogout();
      account = null;
      devices = undefined;
      setting("cloudDevice", null);
    } catch (e) {
      error = errText(e);
    }
  }
</script>

<section class="card" aria-label={t("account.title")}>
  <h3>{t("account.title")}</h3>
  <p class="warn" role="note">{t("account.warn")}</p>
  <p class="muted note">{t("account.note")}</p>

  <form onsubmit={submit} autocomplete="off">
    {#if needsPassword}
      {#if account?.expired}<p class="err" role="alert">{t("account.expired")}</p>{/if}
      <div class="row">
        <label class="field">{t("account.region")}
          <select bind:value={region}><option value="eu">{t("account.regionEu")}</option><option value="us">{t("account.regionUs")}</option></select>
        </label>
        <label class="field">{t("account.email")}<input type="email" bind:value={email} required autocomplete="off" /></label>
        <label class="field">{t("account.password")}<input type="password" bind:value={password} required autocomplete="off" /></label>
      </div>
    {:else if account}
      <p>{t("account.connected", { email: account.email, region: account.region.toUpperCase() })}</p>
      <label class="field">{t("account.device")}
        <select value={chosen?.id ?? ""} onchange={(e) => pick(e.currentTarget.value)}>
          <option value="">{t("account.choose")}</option>
          {#if chosen && !devices?.some((d) => d.devid === chosen.id)}<option value={chosen.id}>{chosen.name}</option>{/if}
          {#each devices ?? [] as d}<option value={d.devid}>{label(d)}</option>{/each}
        </select>
      </label>
      {#if devices && !devices.length}<p class="muted">{t("account.noDevices")}</p>{/if}
      {#if chosen}
        <p class="muted note">{t("account.enableHint")}</p>
        <button class="btn" type="button" onclick={enableLocal} disabled={enabling}>{enabling ? t("account.enabling") : t("account.enableLocal")}</button>
        {#if enableMsg}<p class="ok" role="status">{enableMsg}</p>{/if}
        <label class="check auto"><input type="checkbox" checked={app.settings.autoReenable} onchange={(e) => setting("autoReenable", e.currentTarget.checked)} />{t("account.autoReenable")}</label>
        <p class="muted note ct">{t("account.ctHint")}</p>
        <label class="field">{t("account.ctMeter")}
          <select value={meter ?? ""} onchange={(e) => setting("ctReselectMeter", e.currentTarget.value === "" ? undefined : Number(e.currentTarget.value))}>
            <option value="">{t("account.ctOff")}</option>
            {#each meters as name, code}<option value={code}>{name}</option>{/each}
          </select>
        </label>
        {#if needsMac}
          <label class="field">{t("account.ctMac")}
            <input type="text" maxlength="12" placeholder="aabbccddeeff" value={app.settings.ctReselectMac ?? ""} oninput={(e) => setting("ctReselectMac", e.currentTarget.value.trim() || undefined)} autocomplete="off" />
          </label>
          {#if !macOk}<p class="err" role="alert">{t("account.ctMacBad")}</p>{/if}
        {/if}
        {#if isIpMeter}<p class="muted note">{t("account.ctIpNote")}</p>{/if}
        <button class="btn" type="button" onclick={reselect} disabled={reselecting || !macOk}>{reselecting ? t("account.enabling") : t("account.ctNow")}</button>
        {#if reselectMsg}<p class="ok ct-out" role="status">{reselectMsg}</p>{/if}
      {/if}
    {/if}
    {#if error}<p class="err" role="alert">{error}</p>{/if}
    <div class="row actions">
      <button class="btn primary" type="submit" disabled={loading}>{loading ? t("account.signingIn") : needsPassword ? t("account.signIn") : t("account.refresh")}</button>
      {#if account}<button class="btn" type="button" onclick={logout}>{t("account.signOut")}</button>{/if}
    </div>
  </form>
</section>

<style>
  .warn { padding: 10px 14px; margin-bottom: 8px; border: 1px solid var(--border); border-radius: 8px; background: var(--surface-2); }
  .note { margin-bottom: 14px; font-size: var(--text-sm); }
  .actions { margin-top: 14px; }
  .err { margin-top: 10px; color: var(--danger); }
  .ok { margin-top: 10px; color: var(--ok, var(--accent)); }
  .ct-out { font-family: ui-monospace, monospace; font-size: var(--text-sm); word-break: break-all; color: var(--text); }
  .ct { margin-top: 14px; margin-bottom: 8px; }
  .auto { display: flex; gap: 8px; align-items: center; margin-top: 12px; }
</style>
