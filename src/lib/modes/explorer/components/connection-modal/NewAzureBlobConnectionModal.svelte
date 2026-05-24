<script lang="ts">
  import Modal from '$lib/shared/primitives/Modal.svelte';
  import { createConnection, updateConnection, setSecret, getSecret } from '$lib/modes/explorer/commands';
  import { loadExplorerConnections } from '$lib/modes/explorer/stores';
  import { showToast } from '$lib/shared/primitives/toast';
  import { errorToast, friendlyError } from '$lib/utils/errors';
  import type { ExplorerConnection } from '$lib/modes/explorer/types';

  interface Props {
    show: boolean;
    editing?: ExplorerConnection | null;
    onclose?: () => void;
  }

  let { show = $bindable(), editing = null, onclose }: Props = $props();
  const isEdit = $derived(!!editing);

  let name = $state('');
  let account = $state('');
  let container = $state('');
  let authKind = $state<'shared_key' | 'sas' | 'connection_string'>('shared_key');
  let secretValue = $state('');

  let saving = $state(false);

  let lastFilledId = $state<string | null>(null);
  $effect(() => {
    if (show && editing && editing.id !== lastFilledId) {
      lastFilledId = editing.id;
      name = editing.name ?? '';
      account = editing.azureAccount ?? '';
      container = editing.azureContainer ?? '';
      authKind = (editing.azureAuthKind as typeof authKind) ?? 'shared_key';
      secretValue = '';
      const secretName =
        authKind === 'shared_key' ? 'shared_key' :
        authKind === 'sas' ? 'sas_token' :
        'connection_string';
      getSecret(editing.id, secretName).then((v) => { if (v != null) secretValue = v; }).catch(() => {});
    }
    if (!show) lastFilledId = null;
  });

  function resetForm() {
    name = '';
    account = '';
    container = '';
    authKind = 'shared_key';
    secretValue = '';
  }

  async function handleSave() {
    if (!name.trim()) { showToast('Name is required', 'error'); return; }
    if (!account.trim()) { showToast('Account is required', 'error'); return; }
    if (!container.trim()) { showToast('Container is required', 'error'); return; }
    if (!isEdit && !secretValue.trim()) { showToast('Credential is required', 'error'); return; }
    saving = true;
    try {
      const payload: ExplorerConnection = {
        id: editing?.id ?? '',
        name: name.trim(),
        kind: 'azure_blob',
        accentColor: editing?.accentColor ?? null,
        lastUsedAt: editing?.lastUsedAt ?? null,
        createdAt: editing?.createdAt ?? '',
        sshProfileId: null,
        sftpWorkingDir: null,
        host: null, port: null, username: null, authType: null, keyPath: null,
        ftpPassive: 1, ftpTls: null,
        s3Preset: null, s3Endpoint: null, s3Region: null, s3Bucket: null, s3PathStyle: 0,
        azureAccount: account.trim(),
        azureContainer: container.trim(),
        azureAuthKind: authKind,
      };
      let connId: string;
      if (isEdit && editing) {
        await updateConnection(payload);
        connId = editing.id;
      } else {
        const created = await createConnection(payload);
        connId = created.id;
      }
      if (secretValue.trim()) {
        const secretName =
          authKind === 'shared_key' ? 'shared_key' :
          authKind === 'sas' ? 'sas_token' :
          'connection_string';
        await setSecret(connId, secretName, secretValue.trim());
      }
      await loadExplorerConnections();
      showToast(isEdit ? 'Azure Blob connection updated' : 'Azure Blob connection saved', 'success');
      resetForm();
      show = false;
      onclose?.();
    } catch (e: any) {
      errorToast('Save failed', e);
    } finally {
      saving = false;
    }
  }

  const labels = {
    shared_key: 'Account key',
    sas: 'SAS token',
    connection_string: 'Connection string',
  } as const;

  const placeholders = {
    shared_key: 'paste account key',
    sas: '?sv=...&sig=...',
    connection_string: 'DefaultEndpointsProtocol=https;AccountName=...',
  } as const;
</script>

<Modal bind:show title={isEdit ? 'Edit Azure Blob connection' : 'New Azure Blob connection'} width="520px" {onclose}>
  <div class="form">
    <label class="row">
      <span>Name</span>
      <input class="inp" type="text" bind:value={name} placeholder="e.g. App storage" />
    </label>

    <label class="row">
      <span>Storage account</span>
      <input class="inp" type="text" bind:value={account} placeholder="mystorageacct" />
    </label>

    <label class="row">
      <span>Container</span>
      <input class="inp" type="text" bind:value={container} placeholder="my-container" />
    </label>

    <label class="row">
      <span>Authentication</span>
      <select class="inp" bind:value={authKind}>
        <option value="shared_key">Shared key</option>
        <option value="sas">SAS token</option>
        <option value="connection_string">Connection string</option>
      </select>
    </label>

    <label class="row">
      <span>{labels[authKind]}</span>
      <input class="inp" type="password" bind:value={secretValue} placeholder={placeholders[authKind]} />
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
  .inp {
    background: var(--surface-hover);
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
