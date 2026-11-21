<script lang="ts">
  import { onMount } from "svelte";
  import { cloudDevices, cloudLogin, cloudLogout, cloudStatus, type CloudDevice, type Region } from "../lib/api";
  import { errText } from "../lib/state.svelte";

  const EXPIRED = "session expirée";

  let account = $state<{ email: string; region: string } | null>(null);
  let expired = $state(false);
  let email = $state("");
  let region = $state<Region>("eu");
  let password = $state("");
  let devices = $state<CloudDevice[]>();
  let error = $state("");
  let loading = $state(false);

  const needsPassword = $derived(!account || expired);
  const n = (v: number | null, unit: string) => (v == null ? "—" : `${v.toLocaleString("fr-FR", { maximumFractionDigits: 1 })} ${unit}`);

  onMount(async () => {
    try {
      account = await cloudStatus();
      if (account) {
        email = account.email;
        region = account.region === "us" ? "us" : "eu";
      }
    } catch (e) {
      error = errText(e);
    }
  });

  async function fetchDevices() {
    try {
      devices = await cloudDevices();
      expired = false;
    } catch (e) {
      const m = errText(e);
      expired = m === EXPIRED;
      devices = undefined;
      error = expired ? "Session expirée : saisissez à nouveau votre mot de passe." : m;
    }
  }

  async function submit(e: Event) {
    e.preventDefault();
    error = "";
    loading = true;
    try {
      if (needsPassword) {
        await cloudLogin(email.trim(), password, region);
        account = { email: email.trim(), region };
        expired = false;
      }
      password = "";
      await fetchDevices();
    } catch (e) {
      password = "";
      error = errText(e);
    } finally {
      loading = false;
    }
  }

  async function logout() {
    error = "";
    try {
      await cloudLogout();
      account = null;
      expired = false;
      devices = undefined;
    } catch (e) {
      error = errText(e);
    }
  }
</script>

<section class="card" aria-label="Compte Marstek">
  <h3>Compte Marstek</h3>
  <p class="warn" role="note">
    Cette fonction utilise l'API non officielle du cloud Marstek. La connexion peut déconnecter l'application mobile Marstek de votre compte. Lecture seule, à la demande : le mot de passe n'est jamais enregistré, seul un jeton de session l'est, dans le Gestionnaire d'identifiants Windows.
  </p>

  <form onsubmit={submit} autocomplete="off">
    {#if account && !expired}
      <p>Connecté : <strong>{account.email}</strong> ({account.region.toUpperCase()})</p>
    {:else}
      <div class="row">
        <label class="field">Région
          <select bind:value={region}><option value="eu">Europe</option><option value="us">États-Unis</option></select>
        </label>
        <label class="field">E-mail<input type="email" bind:value={email} required autocomplete="off" /></label>
        <label class="field">Mot de passe<input type="password" bind:value={password} required autocomplete="off" /></label>
      </div>
    {/if}
    {#if error}<p class="err" role="alert">{error}</p>{/if}
    <div class="row actions">
      <button class="btn primary" type="submit" disabled={loading}>{loading ? "Récupération…" : needsPassword ? "Se connecter et récupérer" : "Récupérer"}</button>
      {#if account}<button class="btn" type="button" onclick={logout}>Déconnexion</button>{/if}
    </div>
  </form>

  {#if devices}
    <h3 class="list-title">Appareils du compte</h3>
    {#if devices.length}
      <table>
        <thead><tr><th>Nom</th><th>Type</th><th>Version</th><th>SOC</th><th>Charge</th><th>Décharge</th><th>Capacité</th><th>État</th></tr></thead>
        <tbody>
          {#each devices as d}
            <tr>
              <td>{d.name ?? d.devid}</td><td>{d.type ?? "—"}</td><td class="num">{d.version ?? "—"}</td>
              <td class="num">{n(d.soc, "%")}</td><td class="num">{n(d.charge, "kWh")}</td><td class="num">{n(d.discharge, "kWh")}</td>
              <td class="num">{n(d.capacityKwh, "kWh")}</td><td>{d.status ?? "—"}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    {:else}
      <p class="muted">Aucun appareil sur ce compte.</p>
    {/if}
  {/if}
</section>

<style>
  .warn { padding: 10px 14px; margin-bottom: 14px; border: 1px solid var(--border); border-radius: 8px; background: var(--surface-2); }
  .actions { margin-top: 14px; }
  .err { margin-top: 10px; color: var(--danger); }
  .list-title { margin: 18px 0 8px; }
</style>
