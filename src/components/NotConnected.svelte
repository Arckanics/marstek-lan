<script lang="ts">
  import { app } from "../lib/state.svelte";
  let { hint = "Choisissez une batterie dans l'onglet Appareils pour afficher ses données." } = $props();
</script>

<div class="empty">
  <svg viewBox="0 0 24 24" aria-hidden="true">
    <rect x="3" y="8" width="16" height="9" rx="2" /><path d="M21 11v3M7 8V6M13 8V6" /><path d="M8 12.5h4" />
  </svg>
  <strong>Aucun appareil connecté</strong>
  <p>{hint}</p>
  <button class="btn primary" onclick={() => (app.tab = "devices")}>Choisir un appareil</button>
</div>
