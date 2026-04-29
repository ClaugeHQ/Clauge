<script lang="ts">
  interface FormField {
    key: string;
    value: string;
    enabled: boolean;
  }

  let { body = '[]', onchange }: {
    body: string;
    onchange: (body: string) => void;
  } = $props();

  let fields = $state<FormField[]>([]);
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
          enabled: f.enabled ?? true,
        }));
      } else {
        fields = [{ key: '', value: '', enabled: true }];
      }
    } catch {
      fields = [{ key: '', value: '', enabled: true }];
    }
  });

  function emit() {
    localChange = true;
    onchange(JSON.stringify(fields.map(f => ({ key: f.key, value: f.value, enabled: f.enabled }))));
  }

  function updateField(idx: number, field: keyof FormField, value: string | boolean) {
    fields[idx] = { ...fields[idx], [field]: value };
    fields = [...fields];
    emit();
  }

  function addRow() {
    fields = [...fields, { key: '', value: '', enabled: true }];
    emit();
  }

  function removeRow(idx: number) {
    if (fields.length <= 1) {
      fields = [{ key: '', value: '', enabled: true }];
    } else {
      fields = fields.filter((_, i) => i !== idx);
    }
    emit();
  }
</script>

<div class="form-kv">
  <div class="form-kv-rows">
    {#each fields as field, i (i)}
      <div class="form-kv-row">
        <input
          type="checkbox"
          class="form-kv-check"
          checked={field.enabled}
          onchange={() => updateField(i, 'enabled', !field.enabled)}
        />
        <input
          class="form-kv-input form-kv-key"
          type="text"
          placeholder="Key"
          value={field.key}
          oninput={(e) => updateField(i, 'key', (e.target as HTMLInputElement).value)}
        />
        <input
          class="form-kv-input form-kv-val"
          type="text"
          placeholder="Value"
          value={field.value}
          oninput={(e) => updateField(i, 'value', (e.target as HTMLInputElement).value)}
        />
        <button class="form-kv-del" onclick={() => removeRow(i)} title="Remove">
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
        </button>
      </div>
    {/each}
  </div>
  <button class="form-kv-add" onclick={addRow}>+ Add field</button>
</div>

<style>
  .form-kv {
    display: flex;
    flex-direction: column;
    flex: 1;
    overflow-y: auto;
    padding: 8px 10px;
  }
  .form-kv::-webkit-scrollbar { width: 3px; }
  .form-kv::-webkit-scrollbar-thumb { background: var(--b1); border-radius: 2px; }

  .form-kv-rows {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .form-kv-row {
    display: flex;
    align-items: center;
    gap: 6px;
    min-height: 30px;
  }

  .form-kv-check {
    width: 14px;
    height: 14px;
    flex-shrink: 0;
    accent-color: var(--acc);
    cursor: pointer;
  }

  .form-kv-input {
    height: 28px;
    padding: 0 8px;
    border: 1px solid var(--b1);
    border-radius: 5px;
    background: transparent;
    color: var(--t1);
    font-size: 12px;
    font-family: var(--mono);
    outline: none;
    transition: border-color 0.15s;
  }
  .form-kv-input:focus { border-color: var(--acc); }
  .form-kv-input::placeholder { color: var(--t4); }

  .form-kv-key { flex: 1; min-width: 0; }
  .form-kv-val { flex: 2; min-width: 0; }

  .form-kv-del {
    width: 22px;
    height: 22px;
    display: flex;
    align-items: center;
    justify-content: center;
    border: none;
    background: transparent;
    color: var(--t4);
    border-radius: 4px;
    cursor: pointer;
    flex-shrink: 0;
    transition: color 0.1s, background 0.1s;
  }
  .form-kv-del:hover { color: var(--err); background: rgba(255,255,255,0.05); }

  .form-kv-add {
    margin-top: 6px;
    align-self: flex-start;
    padding: 4px 10px;
    border: 1px dashed var(--b1);
    border-radius: 5px;
    background: transparent;
    color: var(--t3);
    font-size: 11px;
    font-family: var(--ui);
    cursor: pointer;
    transition: color 0.1s, border-color 0.1s;
  }
  .form-kv-add:hover { color: var(--t1); border-color: var(--b2); }
</style>
