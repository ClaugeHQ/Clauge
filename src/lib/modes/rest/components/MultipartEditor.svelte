<script lang="ts">
  import { open } from '@tauri-apps/plugin-dialog';

  interface MultipartField {
    key: string;
    value: string;
    type: 'text' | 'file';
    filePath: string;
    fileName: string;
    enabled: boolean;
  }

  let { body = '[]', onchange }: {
    body: string;
    onchange: (body: string) => void;
  } = $props();

  let fields = $state<MultipartField[]>([]);
  let localChange = false;

  $effect(() => {
    if (localChange) {
      localChange = false;
      return;
    }
    try {
      const parsed = JSON.parse(body);
      if (Array.isArray(parsed) && parsed.length > 0) {
        fields = parsed.map((f: any) => ({
          key: f.key ?? '',
          value: f.value ?? '',
          type: f.type ?? 'text',
          filePath: f.filePath ?? '',
          fileName: f.fileName ?? '',
          enabled: f.enabled ?? true,
        }));
      } else {
        fields = [{ key: '', value: '', type: 'text', filePath: '', fileName: '', enabled: true }];
      }
    } catch {
      fields = [{ key: '', value: '', type: 'text', filePath: '', fileName: '', enabled: true }];
    }
  });

  function emit() {
    localChange = true;
    onchange(JSON.stringify(fields.map(f => ({
      key: f.key, value: f.value, type: f.type,
      filePath: f.filePath, fileName: f.fileName, enabled: f.enabled,
    }))));
  }

  function updateField(idx: number, field: string, value: string | boolean) {
    (fields[idx] as any)[field] = value;
    fields = [...fields];
    emit();
  }

  function changeType(idx: number, newType: 'text' | 'file') {
    fields[idx].type = newType;
    if (newType === 'text') {
      fields[idx].filePath = '';
      fields[idx].fileName = '';
    } else {
      fields[idx].value = '';
    }
    fields = [...fields];
    emit();
  }

  async function pickFile(idx: number) {
    const selected = await open({ multiple: false });
    if (selected) {
      const path = String(selected);
      const name = path.split('/').pop() ?? 'file';
      fields[idx].filePath = String(path);
      fields[idx].fileName = name;
      fields = [...fields];
      emit();
    }
  }

  function addRow() {
    fields = [...fields, { key: '', value: '', type: 'text', filePath: '', fileName: '', enabled: true }];
    emit();
  }

  function removeRow(idx: number) {
    if (fields.length <= 1) {
      fields = [{ key: '', value: '', type: 'text', filePath: '', fileName: '', enabled: true }];
    } else {
      fields = fields.filter((_, i) => i !== idx);
    }
    emit();
  }
</script>

<div class="mp-editor">
  <div class="mp-rows">
    {#each fields as field, i (i)}
      <div class="mp-row">
        <input
          type="checkbox"
          class="mp-check"
          checked={field.enabled}
          onchange={() => updateField(i, 'enabled', !field.enabled)}
        />
        <input
          class="mp-input mp-key"
          type="text"
          placeholder="Key"
          value={field.key}
          oninput={(e) => updateField(i, 'key', (e.target as HTMLInputElement).value)}
        />
        <select
          class="mp-type-sel"
          value={field.type}
          onchange={(e) => changeType(i, (e.target as HTMLSelectElement).value as 'text' | 'file')}
        >
          <option value="text">Text</option>
          <option value="file">File</option>
        </select>
        {#if field.type === 'file'}
          <button class="mp-file-btn" onclick={() => pickFile(i)}>
            {field.fileName || 'Choose File'}
          </button>
        {:else}
          <input
            class="mp-input mp-val"
            type="text"
            placeholder="Value"
            value={field.value}
            oninput={(e) => updateField(i, 'value', (e.target as HTMLInputElement).value)}
          />
        {/if}
        <button class="mp-del" onclick={() => removeRow(i)} title="Remove">
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
        </button>
      </div>
    {/each}
  </div>
  <button class="mp-add" onclick={addRow}>+ Add field</button>
</div>

<style>
  .mp-editor {
    display: flex;
    flex-direction: column;
    flex: 1;
    overflow-y: auto;
    padding: 8px 10px;
  }
  .mp-editor::-webkit-scrollbar { width: 3px; }
  .mp-editor::-webkit-scrollbar-thumb { background: var(--b1); border-radius: 2px; }

  .mp-rows { display: flex; flex-direction: column; gap: 4px; }

  .mp-row {
    display: flex;
    align-items: center;
    gap: 6px;
    min-height: 30px;
  }

  .mp-check {
    width: 14px; height: 14px; flex-shrink: 0;
    accent-color: var(--acc); cursor: pointer;
  }

  .mp-input {
    height: 28px; padding: 0 8px;
    border: 1px solid var(--b1); border-radius: 5px;
    background: transparent; color: var(--t1);
    font-size: 12px; font-family: var(--mono);
    outline: none; transition: border-color 0.15s;
  }
  .mp-input:focus { border-color: var(--acc); }
  .mp-input::placeholder { color: var(--t4); }

  .mp-key { flex: 1; min-width: 0; }
  .mp-val { flex: 2; min-width: 0; }

  .mp-type-sel {
    height: 28px; padding: 0 4px;
    border: 1px solid var(--b1); border-radius: 5px;
    background: transparent; color: var(--t3);
    font-size: 11px; font-family: var(--mono);
    cursor: pointer; outline: none; flex-shrink: 0;
    width: 56px;
  }
  .mp-type-sel:focus { border-color: var(--acc); }

  .mp-file-btn {
    flex: 2; min-width: 0;
    height: 28px; padding: 0 8px;
    border: 1px dashed var(--b1); border-radius: 5px;
    background: transparent; color: var(--t3);
    font-size: 11px; font-family: var(--mono);
    cursor: pointer; text-align: left;
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
    transition: border-color 0.15s, color 0.15s;
  }
  .mp-file-btn:hover { border-color: var(--acc); color: var(--t1); }

  .mp-del {
    width: 22px; height: 22px;
    display: flex; align-items: center; justify-content: center;
    border: none; background: transparent;
    color: var(--t4); border-radius: 4px;
    cursor: pointer; flex-shrink: 0;
    transition: color 0.1s, background 0.1s;
  }
  .mp-del:hover { color: var(--err); background: rgba(255,255,255,0.05); }

  .mp-add {
    margin-top: 6px; align-self: flex-start;
    padding: 4px 10px;
    border: 1px dashed var(--b1); border-radius: 5px;
    background: transparent; color: var(--t3);
    font-size: 11px; font-family: var(--ui);
    cursor: pointer; transition: color 0.1s, border-color 0.1s;
  }
  .mp-add:hover { color: var(--t1); border-color: var(--b2); }
</style>
