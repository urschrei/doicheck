<script>
  let { documents = [], selected, onselect, onsettings, ondelete, onhelp } = $props();

  function meta(status) {
    if (status === "incomplete") return { glyph: "↻", colour: "var(--sev-incomplete)", title: "Interrupted - re-check failures" };
    if (status === "has-issues") return { glyph: "●", colour: "var(--sev-warn)", title: "Has issues" };
    if (status === "failed") return { glyph: "●", colour: "var(--sev-fail)", title: "Check failed" };
    return { glyph: "●", colour: "var(--sev-ok)", title: "Clean" };
  }
</script>

<aside class="sidebar">
  <div class="head">
    <span class="title">Documents</span>
    <span class="actions">
      <button class="gear" onclick={() => onhelp?.()} title="Help and about" aria-label="Help">?</button>
      <button class="gear" onclick={() => onsettings?.()} title="Settings" aria-label="Settings">&#9881;</button>
    </span>
  </div>
  <ul>
    {#each documents as d (d.fingerprint)}
      <li class="row" class:selected={d.fingerprint === selected}>
        <button class="rowmain" onclick={() => onselect?.(d.fingerprint)}>
          <span class="status" style="color:{meta(d.status).colour}" title={meta(d.status).title}>{meta(d.status).glyph}</span>
          <span class="name">{d.filename}</span>
          <span class="when">{d.last_checked}</span>
        </button>
        <button class="del" title="Remove document" aria-label="Remove document" onclick={() => ondelete?.(d.fingerprint)}>&#10005;</button>
      </li>
    {/each}
  </ul>
</aside>

<style>
  .sidebar { background: var(--bg-sidebar); border-right: 1px solid var(--border); overflow: auto; }
  .head { display: flex; align-items: center; justify-content: space-between; padding: 8px 10px; }
  .title { text-transform: uppercase; font-size: 10px; color: var(--text-muted); }
  .gear { border: 0; background: transparent; cursor: pointer; font-size: 16px; color: var(--text); }
  .actions { display: flex; gap: 4px; }
  ul { list-style: none; margin: 0; padding: 0; }
  .row { display: flex; align-items: stretch; }
  .row:hover { background: var(--bg-hover); }
  .row.selected, .row.selected:hover { background: var(--sel-bg); }
  .row.selected .name, .row.selected .when { color: var(--sel-fg); }
  .row.selected .del { color: var(--sel-fg); }
  .rowmain { display: grid; grid-template-columns: 16px minmax(0, 1fr); gap: 4px; padding: 6px 10px; flex: 1; min-width: 0; border: 0; background: transparent; text-align: left; cursor: pointer; font: inherit; color: var(--text); }
  .name { font-weight: 600; overflow-wrap: anywhere; }
  .when { grid-column: 2; color: var(--text-muted); font-size: 11px; overflow-wrap: anywhere; }
  .del { flex: none; align-self: flex-start; margin-top: 6px; border: 0; background: transparent; color: var(--text-muted); cursor: pointer; padding: 0 10px; font-size: 12px; }
  .del:hover { color: var(--danger); }
</style>
