import { Postgres, type PostgresConfig } from "./pg";

export function postgres(id: string, config: PostgresConfig) {
  return new Postgres(id, config);
}
