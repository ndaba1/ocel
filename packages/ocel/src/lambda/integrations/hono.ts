import {
  Lambda,
  type LambdaConfig,
  type TriggerMap,
  type TriggerUnion,
} from "../lambda";
import type { Hono } from "hono";
import { handle } from "hono/aws-lambda";

export function lambda<T extends keyof TriggerMap>(
  id: string,
  app: Hono,
  config?: Omit<LambdaConfig<T>, "trigger" | "handler">,
) {
  const handler = handle(app);

  return new Lambda(id, {
    ...config,
    handler: handler as any,
    trigger: {
      type: "url",
      config: {},
    } as Extract<TriggerUnion, { type: T }>,
  });
}
