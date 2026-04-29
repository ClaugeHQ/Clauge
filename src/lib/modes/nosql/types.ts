export interface NoSqlConnectionConfig {
  name: string;
  driver: 'mongodb' | 'redis';
  connectionString: string;
  host: string;
  port: number;
  database?: string;
  username?: string;
  password?: string;
  ssl: boolean;
  directConnection?: boolean;
}

export interface NoSqlConnection {
  id: string;
  name: string;
  driver: 'mongodb' | 'redis';
  connectionString: string;
  host: string;
  port: number;
  databaseName: string;
  username: string;
  password: string;
  ssl: number;
  directConnection: number;
  sortOrder: number;
  createdAt: string;
  updatedAt: string;
}

export interface NoSqlQueryResult {
  documents: any[];
  totalCount?: number;
  durationMs: number;
}

export interface RedisKeyInfo {
  key: string;
  keyType: string;
  ttl: number;
}

export interface RedisValue {
  key: string;
  keyType: string;
  value: any;
  ttl: number;
}
