import type { ZodType } from "zod";
import { Workflow, type WorkflowConfig } from "./workflow";

export function workflow<TInput extends ZodType = never>(
  id: string,
  config: WorkflowConfig<TInput>
) {
  return new Workflow<TInput>(id, config);
}
