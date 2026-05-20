<script>
  let { documents = [], onselect, onsettings } = $props();
  const dot = (status) =>
    status === "has-issues" ? "#febc2e" : status === "failed" ? "#ff5f57" : "#28c840";
</script>

<aside class="sidebar">
  <div class="head">
    <span class="title">Documents</span>
    <button class="gear" onclick={() => onsettings?.()} title="Settings" aria-label="Settings">&#9881;</button>
  </div>
  <ul>
    {#each documents as d (d.fingerprint)}
      <li>
        <button class="row" onclick={() => onselect?.(d.fingerprint)}>
          <span class="status" style="color:{dot(d.status)}">&#9679;</span>
          <span class="name">{d.filename}</span>
          <span class="when">{d.last_checked}</span>
        </button>
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
  li { margin: 0; }
  .row { display: grid; grid-template-columns: 14px 1fr; gap: 4px; padding: 6px 10px; width: 100%; border: 0; background: transparent; text-align: left; cursor: pointer; font: inherit; }
  .row:hover { background: #ececec; }
  .name { font-weight: 600; }
  .when { grid-column: 2; color: #888; font-size: 11px; }
</style>
