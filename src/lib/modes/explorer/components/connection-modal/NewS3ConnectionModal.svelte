<script lang="ts">
  import Modal from '$lib/shared/primitives/Modal.svelte';
  import { createConnection, updateConnection, setSecret, getSecret } from '$lib/modes/explorer/commands';
  import { loadExplorerConnections } from '$lib/modes/explorer/stores';
  import { showToast } from '$lib/shared/primitives/toast';
  import type { ExplorerConnection } from '$lib/modes/explorer/types';

  interface Props {
    show: boolean;
    editing?: ExplorerConnection | null;
    onclose?: () => void;
  }

  let { show = $bindable(), editing = null, onclose }: Props = $props();
  const isEdit = $derived(!!editing);

  // Frontend mirror of `s3_presets.rs`. Adding a new S3-compatible service
  // means appending here AND in the Rust file.
  const PRESETS = [
    { key: 'aws',    label: 'Amazon S3',                  endpoint: 'https://s3.{region}.amazonaws.com', regionRequired: true,  pathStyle: false },
    { key: 'r2',     label: 'Cloudflare R2',              endpoint: 'https://{account}.r2.cloudflarestorage.com', regionRequired: false, pathStyle: false },
    { key: 'minio',  label: 'MinIO (self-hosted)',        endpoint: '',                                  regionRequired: false, pathStyle: true  },
    { key: 'wasabi', label: 'Wasabi',                     endpoint: 'https://s3.wasabisys.com',          regionRequired: true,  pathStyle: false },
    { key: 'b2',     label: 'Backblaze B2',               endpoint: 'https://s3.us-west-002.backblazeb2.com', regionRequired: true, pathStyle: true },
    { key: 'gcs',    label: 'Google Cloud (S3 mode)',     endpoint: 'https://storage.googleapis.com',    regionRequired: false, pathStyle: false },
    { key: 'custom', label: 'Custom S3-compatible',       endpoint: '',                                  regionRequired: false, pathStyle: false },
  ] as const;

  let name = $state('');
  let preset = $state<typeof PRESETS[number]['key']>('aws');
  let endpoint = $state('');
  let region = $state('us-east-1');
  let bucket = $state('');
  let accessKey = $state('');
  let secretKey = $state('');
  let pathStyle = $state(false);
  let r2AccountId = $state('');

  // Apply preset defaults whenever the preset changes.
  $effect(() => {
    const p = PRESETS.find((x) => x.key === preset)!;
    pathStyle = p.pathStyle;
    if (p.key === 'aws') {
      endpoint = `https://s3.${region || 'us-east-1'}.amazonaws.com`;
    } else if (p.key === 'r2') {
      endpoint = r2AccountId
        ? `https://${r2AccountId}.r2.cloudflarestorage.com`
        : 'https://{account}.r2.cloudflarestorage.com';
    } else if (p.endpoint) {
      endpoint = p.endpoint;
    }
  });

  let saving = $state(false);

  let lastFilledId = $state<string | null>(null);
  $effect(() => {
    if (show && editing && editing.id !== lastFilledId) {
      lastFilledId = editing.id;
      name = editing.name ?? '';
      preset = (editing.s3Preset as typeof preset) ?? 'aws';
      endpoint = editing.s3Endpoint ?? '';
      region = editing.s3Region ?? 'us-east-1';
      bucket = editing.s3Bucket ?? '';
      pathStyle = editing.s3PathStyle === 1;
      accessKey = '';
      secretKey = '';
      r2AccountId = '';
      getSecret(editing.id, 'access_key').then((v) => { if (v != null) accessKey = v; }).catch(() => {});
      getSecret(editing.id, 'secret_key').then((v) => { if (v != null) secretKey = v; }).catch(() => {});
    }
    if (!show) lastFilledId = null;
  });

  function resetForm() {
    name = '';
    preset = 'aws';
    endpoint = '';
    region = 'us-east-1';
    bucket = '';
    accessKey = '';
    secretKey = '';
    pathStyle = false;
    r2AccountId = '';
  }

  async function handleSave() {
    if (!name.trim()) { showToast('Name is required', 'error'); return; }
    if (!endpoint.trim() || endpoint.includes('{')) {
      showToast('Endpoint URL is required (resolve any placeholders)', 'error'); return;
    }
    if (!bucket.trim()) { showToast('Bucket is required', 'error'); return; }
    if (!accessKey.trim() || !secretKey.trim()) {
      showToast('Access key + secret key are required', 'error'); return;
    }
    saving = true;
    try {
      const payload: ExplorerConnection = {
        id: editing?.id ?? '',
        name: name.trim(),
        kind: 's3',
        accentColor: editing?.accentColor ?? null,
        lastUsedAt: editing?.lastUsedAt ?? null,
        createdAt: editing?.createdAt ?? '',
        sshProfileId: null,
        sftpWorkingDir: null,
        host: null, port: null, username: null, authType: null, keyPath: null,
        ftpPassive: 1, ftpTls: null,
        s3Preset: preset,
        s3Endpoint: endpoint.trim(),
        s3Region: region.trim() || 'us-east-1',
        s3Bucket: bucket.trim(),
        s3PathStyle: pathStyle ? 1 : 0,
        azureAccount: null, azureContainer: null, azureAuthKind: null,
      };
      let connId: string;
      if (isEdit && editing) {
        await updateConnection(payload);
        connId = editing.id;
      } else {
        const created = await createConnection(payload);
        connId = created.id;
      }
      if (accessKey.trim()) await setSecret(connId, 'access_key', accessKey.trim());
      if (secretKey.trim()) await setSecret(connId, 'secret_key', secretKey.trim());
      await loadExplorerConnections();
      showToast(isEdit ? 'S3 connection updated' : 'S3 connection saved', 'success');
      resetForm();
      show = false;
      onclose?.();
    } catch (e: any) {
      showToast(`Save failed: ${e}`, 'error');
    } finally {
      saving = false;
    }
  }
</script>

<Modal bind:show title={isEdit ? 'Edit S3-compatible connection' : 'New S3-compatible connection'} width="540px" {onclose}>
  <div class="form">
    <label class="row">
      <span>Name</span>
      <input class="inp" type="text" bind:value={name} placeholder="e.g. Logs bucket" />
    </label>

    <label class="row">
      <span>Service</span>
      <select class="inp" bind:value={preset}>
        {#each PRESETS as p (p.key)}
          <option value={p.key}>{p.label}</option>
        {/each}
      </select>
    </label>

    {#if preset === 'r2'}
      <label class="row">
        <span>Cloudflare account ID</span>
        <input class="inp" type="text" bind:value={r2AccountId} placeholder="abc123def456..." />
      </label>
    {/if}

    <label class="row">
      <span>Endpoint URL</span>
      <input class="inp" type="text" bind:value={endpoint} placeholder="https://..." />
    </label>

    <div class="grid2">
      <label class="row">
        <span>Region</span>
        <input class="inp" type="text" bind:value={region} placeholder="us-east-1" />
      </label>
      <label class="row">
        <span>Bucket</span>
        <input class="inp" type="text" bind:value={bucket} placeholder="my-bucket" />
      </label>
    </div>

    <label class="row">
      <span>Access key ID</span>
      <input class="inp" type="text" bind:value={accessKey} />
    </label>
    <label class="row">
      <span>Secret access key</span>
      <input class="inp" type="password" bind:value={secretKey} />
    </label>

    <label class="row checkbox">
      <input type="checkbox" bind:checked={pathStyle} />
      <span>Use path-style addressing (required for MinIO and some self-hosted setups)</span>
    </label>

    <div class="actions">
      <button class="btn" onclick={() => { show = false; onclose?.(); }}>Cancel</button>
      <button class="btn primary" onclick={handleSave} disabled={saving}>
        {saving ? 'Saving…' : isEdit ? 'Save changes' : 'Save connection'}
      </button>
    </div>
  </div>
</Modal>

<style>
  .form { display: flex; flex-direction: column; gap: 10px; }
  .row { display: flex; flex-direction: column; gap: 4px; font-family: var(--ui); font-size: 12px; color: var(--t2); }
  .row.checkbox { flex-direction: row; align-items: center; gap: 8px; cursor: default; font-size: 11.5px; }
  .grid2 { display: grid; grid-template-columns: 1fr 1fr; gap: 10px; }
  .inp {
    background: rgba(255,255,255,0.04);
    border: 1px solid var(--b1);
    border-radius: 5px;
    color: var(--t1);
    height: 32px;
    padding: 0 10px;
    font-size: 13px;
    font-family: var(--mono);
    outline: none;
    transition: border-color 0.12s;
    box-sizing: border-box;
    width: 100%;
    min-width: 0;
    text-overflow: ellipsis;
  }
  .inp:focus { border-color: var(--acc); }
  select.inp {
    -webkit-appearance: none;
    -moz-appearance: none;
    appearance: none;
    padding-right: 28px;
    background-image: url("data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 12 12' fill='none' stroke='%23b0b0c8' stroke-width='1.5' stroke-linecap='round' stroke-linejoin='round'><polyline points='3 5 6 8 9 5'/></svg>");
    background-repeat: no-repeat;
    background-position: right 10px center;
    background-size: 10px 10px;
  }
  .inp[type="number"]::-webkit-inner-spin-button,
  .inp[type="number"]::-webkit-outer-spin-button {
    -webkit-appearance: none;
    appearance: none;
    margin: 0;
  }
  .inp[type="number"] {
    appearance: textfield;
    -moz-appearance: textfield;
  }
  .actions { display: flex; gap: 8px; justify-content: flex-end; margin-top: 8px; }
  .btn {
    padding: 7px 14px;
    border-radius: 6px;
    border: 1px solid var(--b1);
    background: transparent;
    color: var(--t2);
    font-size: 12px;
    font-family: var(--ui);
    cursor: default;
  }
  .btn:hover:not(:disabled) { border-color: var(--b2); color: var(--t1); }
  .btn.primary { background: var(--acc); color: #fff; border-color: transparent; }
  .btn:disabled { opacity: 0.5; }
</style>
