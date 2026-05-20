<script>
  let { documents = [], onselect, onsettings, ondelete } = $props();

  function meta(status) {
    if (status === "incomplete") return { glyph: "↻", colour: "#ff9f0a", title: "Interrupted - re-check failures" };
    if (status === "has-issues") return { glyph: "●", colour: "#febc2e", title: "Has issues" };
    if (status === "failed") return { glyph: "●", colour: "#ff5f57", title: "Check failed" };
    return { glyph: "●", colour: "#28c840", title: "Clean" };
  }
</script>

<aside class="sidebar">
  <div class="head">
    <span class="title">Documents</span>
    <button class="gear" onclick={() => onsettings?.()} title="Settings" aria-label="Settings">&#9881;</button>
  </div>
  <ul>
    {#each documents as d (d.fingerprint)}
      <li class="row">
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
  .sidebar { background: #f7f7f7; border-right: 1px solid #e3e3e3; overflow: auto; }
  .head { display: flex; align-items: center; justify-content: space-between; padding: 8px 10px; }
  .title { text-transform: uppercase; font-size: 10px; color: #888; }
  .gear { border: 0; background: transparent; cursor: pointer; font-size: 13px; }
  ul { list-style: none; margin: 0; padding: 0; }
  .row { display: flex; align-items: stretch; }
  .row:hover { background: #ececec; }
  .rowmain { display: grid; grid-template-columns: 16px 1fr; gap: 4px; padding: 6px 10px; flex: 1; border: 0; background: transparent; text-align: left; cursor: pointer; font: inherit; }
  .name { font-weight: 600; }
  .when { grid-column: 2; color: #888; font-size: 11px; }
  .del { border: 0; background: transparent; color: #aaa; cursor: pointer; padding: 0 10px; font-size: 12px; }
  .del:hover { color: #b00020; }
</style>
