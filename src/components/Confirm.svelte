<script lang="ts">
  let dlg: HTMLDialogElement;
  let text = $state("");
  let title = $state("");
  let okLabel = $state("");
  let resolve: (v: boolean) => void = () => {};

  export function ask(message: string, opts: { title?: string; ok?: string } = {}) {
    text = message;
    title = opts.title ?? "Confirmer l'écriture";
    okLabel = opts.ok ?? "Écrire sur l'appareil";
    dlg.showModal();
    return new Promise<boolean>((r) => (resolve = r));
  }

  function done(v: boolean) {
    dlg.close();
    resolve(v);
  }
</script>

<dialog bind:this={dlg} aria-labelledby="confirm-title" oncancel={() => resolve(false)}>
  <h2 id="confirm-title">{title}</h2>
  <p>{text}</p>
  <div class="actions">
    <button class="btn" onclick={() => done(false)}>Annuler</button>
    <button class="btn primary" onclick={() => done(true)}>{okLabel}</button>
  </div>
</dialog>

<style>
  dialog {
    max-width: 440px; padding: 20px; background: var(--surface); color: var(--text);
    border: 1px solid var(--border); border-radius: var(--radius);
  }
  dialog::backdrop { background: rgb(0 0 0 / 0.45); }
  p { margin: 10px 0 18px; color: var(--muted); white-space: pre-line; }
  .actions { display: flex; justify-content: flex-end; gap: 8px; }
</style>
