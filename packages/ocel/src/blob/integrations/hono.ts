import type { Context } from "hono";
import type { Bucket } from "../bucket";

export function createRouteHandler(bucket: Bucket<any>) {
  return async (c: Context) => {
    const req = c.req.raw;
    const body = (await req.json()) as any;

    const result = await bucket.handleUpload({
      body,
      request: req,
    });

    return c.json(result);
  };
}
