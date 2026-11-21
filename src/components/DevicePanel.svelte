<script lang="ts">
  import { setBle, setDod, setLed } from "../lib/api";
  import { app, write } from "../lib/state.svelte";
  import { fmtKwh } from "../lib/energy";
  import Account from "./Account.svelte";
  import NotConnected from "./NotConnected.svelte";

  const s = $derived(app.snapshot);
  const off = $derived(app.busy);
  let dod = $state(80);

  const rssiText = (r: number | undefined) => (r == null ? "—" : `${r} dBm · ${r >= -60 ? "bon" : r >= -75 ? "moyen" : "faible"}`);
  const v = (x: number | undefined, unit: string, digits = 0) =>
    x == null ? "—" : `${x.toLocaleString("fr-FR", { maximumFractionDigits: digits })} ${unit}`;

  const pvRows = $derived(
    ([1, 2, 3, 4] as const).map((n) => ({
      n,
      p: s?.pv?.[`pv${n}Power`],
      u: s?.pv?.[`pv${n}Voltage`],
      i: s?.pv?.[`pv${n}Current`],
    })),
  );
</script>

<div class="stack">
{#if !app.connectedIp}
  <NotConnected />
{:else}
  <div class="grid2">
    <section class="card" aria-label="Réseau sans fil">
      <h3>Connectivité</h3>
      <dl>
        <dt>Wi-Fi (SSID)</dt><dd>{s?.wifi?.ssid ?? "—"}</dd>
        <dt>Signal Wi-Fi</dt><dd class="num">{rssiText(s?.wifi?.rssi)}</dd>
        <dt>Adresse IP</dt><dd class="num">{s?.wifi?.staIp ?? app.connectedIp}</dd>
        <dt>Adresse MAC Wi-Fi</dt><dd class="num">{s?.wifi?.wifiMac ?? s?.device?.wifiMac ?? "—"}</dd>
        <dt>Bluetooth</dt><dd>{s?.ble?.state ?? "—"}</dd>
        <dt>Adresse MAC Bluetooth</dt><dd class="num">{s?.ble?.bleMac ?? s?.device?.bleMac ?? "—"}</dd>
      </dl>
    </section>

    <section class="card" aria-label="Compteur CT">
      <h3>Compteur CT</h3>
      {#if s?.em?.ctState === 1}
        <dl>
          <dt>État</dt><dd><span class="badge"><span class="dot on"></span>Connecté</span></dd>
          <dt>Phase A</dt><dd class="num">{v(s.em.aPower, "W")}</dd>
          <dt>Phase B</dt><dd class="num">{v(s.em.bPower, "W")}</dd>
          <dt>Phase C</dt><dd class="num">{v(s.em.cPower, "W")}</dd>
          <dt>Total</dt><dd class="num"><strong>{v(s.em.totalPower, "W")}</strong></dd>
        </dl>
      {:else}
        <p class="muted">{s?.em ? "Aucun compteur CT détecté sur cette batterie." : "En attente de la première mesure."}</p>
      {/if}
    </section>

    <section class="card" aria-label="Entrées solaires">
      <h3>Panneaux solaires</h3>
      <table>
        <thead><tr><th>Entrée</th><th>Puissance</th><th>Tension</th><th>Courant</th></tr></thead>
        <tbody>
          {#each pvRows as r}
            <tr><td>PV {r.n}</td><td class="num">{v(r.p, "W")}</td><td class="num">{v(r.u, "V", 1)}</td><td class="num">{v(r.i, "A", 2)}</td></tr>
          {/each}
        </tbody>
      </table>
    </section>

    <section class="card" aria-label="Compteurs d'énergie">
      <h3>Compteurs d'énergie</h3>
      <dl>
        <dt>Solaire produit</dt><dd class="num">{fmtKwh(s?.es?.totalPvEnergy, 100)}</dd>
        <dt>Soutiré au réseau</dt><dd class="num">{fmtKwh(s?.es?.totalGridInputEnergy, 1000)}</dd>
        <dt>Injecté au réseau</dt><dd class="num">{fmtKwh(s?.es?.totalGridOutputEnergy, 1000)}</dd>
        <dt>Consommé (sortie hors réseau)</dt><dd class="num">{fmtKwh(s?.es?.totalLoadEnergy, 1000)}</dd>
        <dt>Compteur CT, importé</dt><dd class="num">{fmtKwh(s?.em?.inputEnergy, 10000)}</dd>
        <dt>Compteur CT, exporté</dt><dd class="num">{fmtKwh(s?.em?.outputEnergy, 10000)}</dd>
      </dl>
    </section>

    <section class="card wide" aria-label="Réglages de l'appareil">
      <h3>Réglages</h3>
      <div class="settings">
        <div class="item">
          <label class="field" for="dod">Profondeur de décharge (DOD) : <strong class="num">{dod} %</strong></label>
          <input id="dod" type="range" min="30" max="88" bind:value={dod} />
          <p class="muted">La valeur en vigueur n'est pas lisible sur l'appareil.</p>
          <button class="btn" disabled={off} onclick={() => write(`DOD ${dod} %`, () => setDod(dod))}>Appliquer</button>
        </div>
        <div class="item">
          <span class="lab">Voyant LED</span>
          <div class="seg" role="group" aria-label="Voyant LED">
            <button disabled={off} onclick={() => write("LED allumée", () => setLed(true))}>Allumer</button>
            <button disabled={off} onclick={() => write("LED éteinte", () => setLed(false))}>Éteindre</button>
          </div>
        </div>
        <div class="item">
          <span class="lab">Diffusion Bluetooth</span>
          <div class="seg" role="group" aria-label="Diffusion Bluetooth">
            <button disabled={off} onclick={() => write("Bluetooth activé", () => setBle(true))}>Activer</button>
            <button disabled={off} onclick={() => write("Bluetooth désactivé", () => setBle(false))}>Désactiver</button>
          </div>
        </div>
      </div>
    </section>
  </div>
{/if}
  <Account />
</div>

<style>
  .wide { grid-column: 1 / -1; }
  .settings { display: grid; grid-template-columns: 2fr 1fr 1fr; gap: 24px; }
  .item { display: grid; gap: 8px; align-content: start; }
  .item .btn, .item .seg { justify-self: start; }
  .lab { font-size: 0.85rem; color: var(--muted); }
  .field strong { color: var(--text); }
</style>
