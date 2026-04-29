import { invoke } from '@tauri-apps/api/core';
import type { NoSqlConnection, NoSqlConnectionConfig, NoSqlQueryResult, RedisKeyInfo, RedisValue } from './types';

// --- Connection management ---

export async function nosqlConnect(config: NoSqlConnectionConfig): Promise<string> {
  return invoke('nosql_connect', { config });
}

export async function nosqlDisconnect(connectionId: string): Promise<void> {
  return invoke('nosql_disconnect', { connectionId });
}

export async function nosqlTestConnection(config: NoSqlConnectionConfig): Promise<string> {
  return invoke('nosql_test_connection', { config });
}

// --- Database / Collection browsing ---

export async function nosqlListDatabases(connectionId: string): Promise<string[]> {
  return invoke('nosql_list_databases', { connectionId });
}

export async function nosqlListCollections(connectionId: string, database: string): Promise<string[]> {
  return invoke('nosql_list_collections', { connectionId, database });
}

// --- Document operations ---

export async function nosqlFindDocuments(
  connectionId: string,
  database: string,
  collection: string,
  filter: string,
  sort: string,
  limit: number,
  skip: number
): Promise<NoSqlQueryResult> {
  return invoke('nosql_find_documents', { connectionId, database, collection, filter, sort, limit, skip });
}

export async function nosqlInsertDocument(
  connectionId: string,
  database: string,
  collection: string,
  document: string
): Promise<string> {
  return invoke('nosql_insert_document', { connectionId, database, collection, document });
}

export async function nosqlUpdateDocument(
  connectionId: string,
  database: string,
  collection: string,
  filter: string,
  update: string
): Promise<number> {
  return invoke('nosql_update_document', { connectionId, database, collection, filter, update });
}

export async function nosqlDeleteDocument(
  connectionId: string,
  database: string,
  collection: string,
  filter: string
): Promise<number> {
  return invoke('nosql_delete_document', { connectionId, database, collection, filter });
}

export async function nosqlCountDocuments(
  connectionId: string,
  database: string,
  collection: string,
  filter: string
): Promise<number> {
  return invoke('nosql_count_documents', { connectionId, database, collection, filter });
}

export async function nosqlAggregate(
  connectionId: string,
  database: string,
  collection: string,
  pipeline: string
): Promise<NoSqlQueryResult> {
  return invoke('nosql_aggregate', { connectionId, database, collection, pipeline });
}

// --- Database & Collection management ---

export async function nosqlCreateCollection(connectionId: string, database: string, collection: string): Promise<void> {
  return invoke('nosql_create_collection', { connectionId, database, collection });
}

export async function nosqlDropDatabase(connectionId: string, database: string): Promise<void> {
  return invoke('nosql_drop_database', { connectionId, database });
}

export async function nosqlDropCollection(connectionId: string, database: string, collection: string): Promise<void> {
  return invoke('nosql_drop_collection', { connectionId, database, collection });
}

export async function nosqlRenameCollection(connectionId: string, database: string, collection: string, newName: string): Promise<void> {
  return invoke('nosql_rename_collection', { connectionId, database, collection, newName });
}

// --- Redis operations ---

export async function redisExecute(connectionId: string, command: string): Promise<string> {
  return invoke('redis_execute', { connectionId, command });
}

export async function redisListKeys(connectionId: string, pattern: string, cursor: number, count: number): Promise<{ keys: RedisKeyInfo[]; nextCursor: number }> {
  return invoke('redis_list_keys', { connectionId, pattern, cursor, count });
}

export async function redisGetKey(connectionId: string, key: string): Promise<RedisValue> {
  return invoke('redis_get_key', { connectionId, key });
}

export async function redisSetKey(connectionId: string, key: string, value: string, ttl?: number): Promise<void> {
  return invoke('redis_set_key', { connectionId, key, value, ttl });
}

export async function redisDeleteKey(connectionId: string, key: string): Promise<void> {
  return invoke('redis_delete_key', { connectionId, key });
}

export async function redisGetInfo(connectionId: string): Promise<string> {
  return invoke('redis_get_info', { connectionId });
}

// --- Saved connections ---

export async function nosqlSaveConnection(config: NoSqlConnectionConfig): Promise<NoSqlConnection> {
  return invoke('nosql_save_connection', { config });
}

export async function nosqlListSavedConnections(): Promise<NoSqlConnection[]> {
  return invoke('nosql_list_saved_connections');
}

export async function nosqlDeleteSavedConnection(id: string): Promise<void> {
  return invoke('nosql_delete_saved_connection', { id });
}

export async function nosqlUpdateSavedConnection(id: string, config: NoSqlConnectionConfig): Promise<NoSqlConnection> {
  return invoke('nosql_update_saved_connection', { id, config });
}
