import { Lambda, type LambdaConfig, type TriggerMap } from "./lambda";

export function lambda<T extends keyof TriggerMap>(
  id: string,
  config: LambdaConfig<T>
) {
  return new Lambda(id, config);
}
