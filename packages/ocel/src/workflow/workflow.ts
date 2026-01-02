import type { z, ZodType } from "zod";
import type { Component } from "../utils/component";
import {
  withDurableExecution,
  type DurableContext,
} from "@aws/durable-execution-sdk-js";
import { getCallerFile } from "../utils/stack";
import { rpc } from "../utils/rpc";
import { SpanStatusCode, trace } from "@opentelemetry/api";

export interface WorkflowConfig<TInput extends ZodType = never> {
  input?: TInput;
  queue?: {
    concurrencyLimit?: number;
  };
  cron?: string;
  run: [TInput] extends [never]
    ? (args: { ctx: DurableContext }) => Promise<void>
    : (args: { input: z.infer<TInput>; ctx: DurableContext }) => Promise<void>;
  link?: Component[];
  retry?:
    | {
        maxAttempts: number;
        backoffRate?: number;
        intervalSeconds?: number;
        maxTimeoutSeconds?: number;
      }
    | false;
}

export class Workflow<TInput extends ZodType = never> implements Component {
  constructor(private id: string, config: WorkflowConfig<TInput>) {
    const { run, input, link, ...rest } = config;
    const links = link?.map((component) => component.__id()) || [];
    const tracer = trace.getTracer("ocel/workflow");

    const handler = (originalEvent: any, originalFnContext: any) => {
      return withDurableExecution(async (event, durableCtx) => {
        const ctx = durableCtx as any;
        const isReplay = ctx.durableExecutionMode === "ReplayMode";
        const ops = originalEvent.InitialExecutionState?.Operations || [];

        const stepCompleted = (name: string) => {
          return (
            isReplay &&
            ops.some((op: any) => op.Name === name && op.Status === "SUCCEEDED")
          );
        };

        return tracer.startActiveSpan(
          isReplay ? `workflow-${id}-replay` : `workflow-${id}`,
          {
            attributes: { workflow_id: id },
          },
          async (rootSpan) => {
            // override context to add tracing
            const instrumentedCtx = new Proxy(durableCtx, {
              get(target, prop, receiver) {
                if (prop === "step") {
                  return async (
                    name: string,
                    fn: () => Promise<any>,
                    opts: any
                  ) => {
                    if (stepCompleted(name)) {
                      return target.step(name, fn, opts);
                    }

                    return tracer.startActiveSpan(
                      `step: ${name}`,
                      async (stepSpan) => {
                        try {
                          console.info(
                            `stepSpan isRecording=${stepSpan.isRecording()} spanContext=${JSON.stringify(
                              stepSpan.spanContext()
                            )}`
                          );

                          const result = await target.step(name, fn, opts);
                          stepSpan.setStatus({ code: SpanStatusCode.OK });
                          return result;
                        } catch (err) {
                          // Only record legitimate errors, not Durable Suspend/Sleep interruptions
                          // (You can filter specifically if you know the Suspend error type)
                          stepSpan.recordException(err as Error);
                          stepSpan.setStatus({ code: SpanStatusCode.ERROR });
                          throw err;
                        } finally {
                          stepSpan.end();
                        }
                      }
                    );
                  };
                }

                if (prop === "wait") {
                  return async (name: string, opts: any) => {
                    if (stepCompleted(name)) {
                      return target.wait(name, opts);
                    }

                    return tracer.startActiveSpan(
                      `wait: ${name}`,
                      async (waitSpan) => {
                        console.info(
                          `waitSpan isRecording=${waitSpan.isRecording()} spanContext=${JSON.stringify(
                            waitSpan.spanContext()
                          )}`
                        );

                        waitSpan.setAttribute("duration_seconds", opts.seconds);
                        waitSpan.setAttribute("workflow.status", "suspending");

                        console.log(
                          "[OTEL-FIX] Closing spans before suspend..."
                        );

                        waitSpan.setStatus({ code: SpanStatusCode.OK });
                        waitSpan.end();

                        rootSpan.setAttribute("workflow.suspend_trigger", name);
                        rootSpan.setStatus({ code: SpanStatusCode.OK });
                        rootSpan.end();

                        const provider = trace.getTracerProvider();
                        // @ts-ignore
                        if (typeof provider.forceFlush === "function") {
                          // @ts-ignore
                          await provider.forceFlush();
                          console.log("[OTEL-FIX] Flush complete.");
                        }

                        await target.wait(name, opts);
                      }
                    );
                  };
                }

                // Default: Return the original property (e.g., ctx.runId, ctx.now)
                return Reflect.get(target, prop, receiver);
              },
            });

            console.log("eventss", {
              originalEvent,
              originalFnContext,
            });

            const executionArn = originalEvent.DurableExecutionArn;
            const parts = executionArn.split("/");
            const executionId = `${parts[parts.length - 2]}/${
              parts[parts.length - 1]
            }`;

            console.log("execution details", {
              event,
              originalCtx: durableCtx,
              executionArn,
              executionId,
            });

            rootSpan.setAttribute("execution_id", executionId);

            try {
              console.info(
                `startRootSpan isRecording=${rootSpan.isRecording()} spanContext=${JSON.stringify(
                  rootSpan.spanContext()
                )}`
              );

              if (input) {
                const parsed = input.parse(event);
                await run({ input: parsed, ctx: instrumentedCtx });
              } else {
                await run({ ctx: instrumentedCtx, input: event });
              }

              rootSpan.setStatus({ code: 1 });
            } catch (error) {
              console.log("Error in workflow:", error);
              throw error;
            } finally {
              console.log("Ending span for workflow:", id);
              console.log("Span details:", rootSpan);

              if (rootSpan.isRecording()) {
                console.log("Ending root span (normal exit)");
                rootSpan.end();
              }
            }
          }
        );
      })(originalEvent, originalFnContext);
    };

    // @ts-expect-error - accessed by cli during build
    this.__handler = handler;
    // @ts-expect-error - accessed by cli during build
    this[Symbol.for("ocel:lambda:id")] = id;

    if (process.env.OCEL_PHASE === "discovery") {
      const source = getCallerFile();

      if (!source) {
        throw new Error(
          "Could not determine caller file for workflow registration"
        );
      }

      rpc.register({
        id: this.id,
        type: "workflow",
        source,
        config: { ...rest, links },
      });
    }
  }

  __id() {
    return this.id;
  }

  trigger(
    payload: [TInput] extends [never] ? {} : z.infer<TInput>,
    opts?: {
      idempotencyKey?: string;
    }
  ) {}

  batchTrigger() {}

  triggerAndWait() {}

  batchTriggerAndWait() {}
}
