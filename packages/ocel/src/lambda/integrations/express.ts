import {
  Lambda,
  type LambdaConfig,
  type TriggerMap,
  type TriggerUnion,
} from "../lambda";
import sls from "serverless-http";
import type { Express } from "express";

export function lambda<T extends keyof TriggerMap>(
  id: string,
  app: Express,
  config?: Omit<LambdaConfig<T>, "trigger" | "handler">
) {
  const handler = sls(app);

  return new Lambda(id, {
    ...config,
    handler,
    trigger: {
      type: "url",
      config: {},
    } as Extract<TriggerUnion, { type: T }>,
  });
}
