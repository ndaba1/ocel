import type { IncomingMessage } from "node:http";
import z from "zod";
import { getCallerFile } from "../utils/stack";
import { rpc } from "../utils/rpc";
import type { HandleUploadBody, TBucketConfig, TUploaderConfig } from "./types";
import { Uploader } from "./uploader";
import { parseReq } from "./parse";
import type { Component } from "../utils/component";

export class Bucket<
  TUploaderShape extends Record<string, TUploaderConfig<any, any>>
> implements Component
{
  public readonly _shape: TUploaderShape = {} as any;
  private uploaders: Record<string, Uploader<any>> = {};

  constructor(
    public readonly name: string,
    args: TBucketConfig<TUploaderShape>
  ) {
    if (process.env.OCEL_PHASE === "discovery") {
      rpc.register({
        id: name,
        type: "bucket",
        source: getCallerFile() || "unknown",
        // TODO:
        config: {},
      });
    }

    this._shape = args.uploaders;

    Object.entries(args.uploaders).forEach(([key, config]) => {
      this.uploaders[key] = new Uploader(this, key, config);
    });
  }

  __id() {
    return this.name;
  }

  __name() {
    const id = process.env[`RESOURCE_${this.name}_BUCKET_NAME`];
    if (!id) {
      throw new Error(
        `Bucket ID for bucket "${this.name}" is not defined. Make sure the bucket is properly configured in your Ocel project.`
      );
    }
    return id;
  }

  upload = async () => {};

  uploadExternal = async () => {};

  handleUpload = async ({
    body,
    request,
  }: {
    request: Request | IncomingMessage;
    body: HandleUploadBody;
  }) => {
    const parsedReq = parseReq(request);
    const hasUploaders = Object.keys(this._shape).length > 0;
    if (!hasUploaders) {
      throw new Error("No uploaders configured for this bucket");
    }

    const querySchema = z.object({
      action: z.enum(["presign", "callback", "poll"]),
      uploader: z.enum(Object.keys(this._shape) as [string, ...string[]]),
      sessionId: z.string().optional(),
    });
    const queryRes = querySchema.safeParse(Object.fromEntries(parsedReq.query));
    if (!queryRes.success) {
      throw new Error(`Invalid query parameters: ${queryRes.error.message}`);
    }

    const query = queryRes.data;
    const uploader = this.uploaders[query.uploader];
    if (!uploader) {
      throw new Error(`Uploader not found: ${query.uploader}`);
    }

    return uploader.handleUpload({
      body,
      request,
      action: query.action,
      sessionId: query.sessionId,
    });
  };
}
