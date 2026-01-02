import type { Component } from "../utils/component";
import { rpc } from "../utils/rpc";
import { getCallerFile } from "../utils/stack";

export type TriggerMap = {
  s3: {
    bucket: string;
    events: string[];
  };
  cron: {
    schedule: string;
  };
  url: {
    streaming?: boolean;
  };
  api: {
    apiId?: string;
    path: string;
    method: "GET" | "POST" | "PUT" | "DELETE" | "PATCH" | "OPTIONS";
  };
};

export type TriggerUnion = {
  [K in keyof TriggerMap]: { type: K; config: TriggerMap[K] };
}[keyof TriggerMap];

export interface LambdaConfig<T extends keyof TriggerMap> {
  handler: (event: any, context: any) => Promise<any>;

  trigger: Extract<TriggerUnion, { type: T }>;
  link?: Component[];
}

export class Lambda<T extends keyof TriggerMap> implements Component {
  constructor(public id: string, config: LambdaConfig<T>) {
    // remove non-serializable stuff
    const { handler, link, ...rest } = config;
    const links = link?.map((component) => component.__id()) || [];

    // @ts-expect-error - accessed by cli during build
    this.__handler = handler;
    // @ts-expect-error - accessed by cli during build
    this[Symbol.for("ocel:lambda:id")] = id;

    if (process.env.OCEL_PHASE === "discovery") {
      const source = getCallerFile();

      if (!source) {
        throw new Error(
          "Could not determine caller file for lambda registration"
        );
      }

      rpc.register({
        id: this.id,
        type: "lambda",
        source,
        config: { ...rest, links },
      });
    }
  }

  __id(): string {
    return this.id;
  }
}
