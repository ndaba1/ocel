import type { Component } from "../utils/component";
import { rpc } from "../utils/rpc";
import { getCallerFile } from "../utils/stack";

export interface PostgresConfig {
  version?: string;
  migrations?: string;
}

export class Postgres implements Component {
  constructor(public id: string, config: PostgresConfig) {
    // remove non-serializable stuff

    if (process.env.OCEL_PHASE === "discovery") {
      const source = getCallerFile();

      rpc.register({
        id: this.id,
        type: "postgres",
        source: source || "unknown",
        config: {},
      });
    }
  }

  __id(): string {
    return this.id;
  }

  async sql(query: string) {
    return {};
  }
}
