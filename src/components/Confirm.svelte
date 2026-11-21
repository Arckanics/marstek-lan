<script lang="ts">
  import type { Slot } from "../lib/api";
  import { dayName, t } from "../lib/i18n.svelte";

  let dlg: HTMLDialogElement;
  let text = $state("");
  let title = $state("");
  let okLabel = $state("");
  let slots = $state<Slot[]>();
  let note = $state("");
  let resolve: (v: boolean) => void = () => {};

  /** `slots` : aperçu des créneaux qui seront écrits. */
  export function ask(message: string, opts: { title?: string; ok?: string; slots?: Slot[]; note?: string } = {}) {
    text = message;
    title = opts.title ?? t("confirm.title");
    okLabel = opts.ok ?? t("confirm.ok");
    slots = opts.slots;
    note = opts.note ?? "";
    dlg.showModal();
    return new Promise<boolean>((r) => (resolve = r));
  }

  function done(v: boolean) {
    dlg.close();
    resolve(v);
  }

  const dayList = (mask: number) => (mask === 127 ? t("common.everyDay") : Array.from({ length: 7 }, (_, i) => i).filter((i) => mask & (1 << i)).map((i) => dayName(i).slice(0, 3)).join(" "));
</script>

<dialog bind:this={dlg} aria-labelledby="confirm-title" oncancel={() => resolve(false)}>
  <h2 id="confirm-title">{title}</h2>
  <p class="msg">{text}</p>
  {#if slots}
    {#if slots.length}
      <table>
        <thead><tr><th>{t("confirm.no")}</th><th>{t("confirm.times")}</th><th>{t("confirm.direction")}</th><th>{t("confirm.power")}</th><th>{t("common.days")}</th></tr></thead>
        <tbody>
          {#each slots as s, i}
            <tr class="k-{s.direction}">
              <td class="num">{i}</td>
              <td class="num">{t("confirm.range", { start: s.start, end: s.end })}</td>
              <td><span class="swatch"></span>{t(s.direction === "charge" ? "dir.charge" : "dir.discharge")}</td>
              <td class="num">{s.power} W</td>
              <td>{dayList(s.days)}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    {:else}
      <p class="msg">{t("confirm.noSlots")}</p>
    {/if}
  {/if}
  {#if note}<p class="note">{note}</p>{/if}
  <div class="actions">
    <button class="btn" onclick={() => done(false)}>{t("common.cancel")}</button>
    <button class="btn primary" onclick={() => done(true)}>{okLabel}</button>
  </div>
</dialog>

<style>
  dialog {
    width: 560px; max-width: calc(100% - 32px); padding: 20px; background: var(--surface); color: var(--text);
    border: 1px solid var(--border); border-radius: var(--radius);
  }
  dialog::backdrop { background: var(--scrim); }
  .msg { margin: 12px 0 16px; color: var(--muted); white-space: pre-line; }
  table { margin-bottom: 16px; }
  td .swatch { display: inline-block; margin-right: 8px; vertical-align: -1px; }
  .note { margin-bottom: 16px; padding: 8px 12px; border-left: 2px solid var(--auto); background: var(--surface-2); font-size: var(--text-sm); }
  .actions { display: flex; justify-content: flex-end; gap: 8px; }
</style>
