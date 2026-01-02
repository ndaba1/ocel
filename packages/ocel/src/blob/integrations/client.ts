import type z from "zod";
import type { Bucket } from "../bucket";
import type { PresignUploadBody, PresignUploadResponse } from "../types";

type InferReturn<TRouter, K extends keyof TRouter> = TRouter[K] extends {
  _def: { output: infer R };
}
  ? R
  : never;

type InferInput<TRouter, K extends keyof TRouter> = TRouter[K] extends {
  _def: { input: infer R };
}
  ? R
  : never;

export function createUploadClient<TBucket extends Bucket<any>>(opts: {
  url: string;
}) {
  return {
    upload: async <K extends keyof TBucket["_shape"]>(
      uploader: K,
      args: InferInput<TBucket["_shape"], K> extends never
        ? {
            files: File[];
            onClientUploadComplete?: (
              res: InferReturn<TBucket["_shape"], K>
            ) => void;
          }
        : {
            files: File[];
            input: z.infer<InferInput<TBucket["_shape"], K>>;
            onClientUploadComplete?: (
              res: InferReturn<TBucket["_shape"], K>
            ) => void;
          }
    ) => {
      const url = opts.url;
      const searchParams = new URLSearchParams();
      searchParams.append("action", "presign");
      searchParams.append("uploader", String(uploader));

      // get presign urls
      const result = (await fetch(`${url}?${searchParams.toString()}`, {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
        },
        body: JSON.stringify({
          files: args.files.map((f) => {
            return {
              name: f.name,
              size: f.size,
              type: f.type,
              lastModified: f.lastModified,
              mimeType: f.type,
            };
          }),
          ...("input" in args ? { input: args.input } : {}),
        } satisfies PresignUploadBody),
      }).then((r) => r.json())) as PresignUploadResponse;

      // upload files to presign urls
      await Promise.all(
        args.files.map((file, index) => {
          if (!result.files[index]?.url) {
            throw new Error(`No presign URL returned for file index ${index}`);
          }

          return fetch(result.files[index].url, {
            method: "PUT",
            headers: {
              "Content-Type": file.type,
            },
            body: file,
          });
        })
      );

      // TODO: poll for completion from server
      // url.searchParams.set("action", "poll");
      // const ev = new EventSource(url.toString());

      // ev.onmessage = (event) => {
      //   const data = JSON.parse(event.data);
      //   if (data.status === "completed") {
      //     ev.close();
      //     args.onClientUploadComplete?.(
      //       data.result as InferReturn<TBucket["_shape"], K>
      //     );
      //   }
      // };
    },
  };
}
