import type { Component } from "../utils/component";
import { rpc } from "../utils/rpc";
import { getCallerFile } from "../utils/stack";
import type {
  Context,
  APIGatewayEvent,
  S3Event,
  LambdaFunctionURLEvent,
} from "aws-lambda";

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
    id?: string;
    path: string;
    method: "GET" | "POST" | "PUT" | "DELETE" | "PATCH" | "OPTIONS" | "*";
  };
};

export type TriggerUnion = {
  [K in keyof TriggerMap]: { type: K; config: TriggerMap[K] };
}[keyof TriggerMap];

export type LambdaHandlerOutput<T extends keyof TriggerMap> = T extends "api"
  ? {
      statusCode: number;
      headers: Record<string, string>;
      body: string;
    }
  : any;

export type LambdaEvent<T extends keyof TriggerMap> = T extends "s3"
  ? S3Event
  : T extends "api"
    ? APIGatewayEvent
    : T extends "url"
      ? LambdaFunctionURLEvent
      : any;

export interface LambdaConfig<T extends keyof TriggerMap> {
  handler: (
    event: LambdaEvent<T>,
    context: Context,
  ) => Promise<LambdaHandlerOutput<T>> | LambdaHandlerOutput<T>;

  trigger: Extract<TriggerUnion, { type: T }>;
  link?: Component[];
}

export class Lambda<T extends keyof TriggerMap> implements Component {
  constructor(
    public id: string,
    config: LambdaConfig<T>,
  ) {
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
          "Could not determine caller file for lambda registration",
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
