import { type ZodType } from "zod";
import type { TBucketConfig, TUploaderConfig } from "./types";
import { Bucket } from "./bucket";

export function bucket<
  TUploaderShape extends Record<string, TUploaderConfig<any, any>>
>(name: string, args: TBucketConfig<TUploaderShape>) {
  return new Bucket<TUploaderShape>(name, args);
}

export function uploader<
  TInput extends ZodType = never,
  TMeta = never,
  TReturn = void
>(
  {
    middleware,
    input,
  }: {
    input?: TInput;
    middleware: TUploaderConfig<TInput, TMeta, TReturn>["onBeforeUpload"];
  },
  config: Omit<
    TUploaderConfig<TInput, TMeta, TReturn>,
    "onBeforeUpload" | "_def" | "input"
  >
) {
  return {
    ...config,
    input,
    onBeforeUpload: middleware,
    _def: {} as any,
  } as TUploaderConfig<TInput, TMeta, TReturn>;
}
