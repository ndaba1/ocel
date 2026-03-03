import type z from "zod";
import type { Bucket } from "../bucket";
import type { PresignUploadBody, PresignUploadResponse } from "../types";

type PollResponse = {
  files: Array<{
    fileKey: string;
    status: "PENDING" | "SUCCESS" | "FAILED";
    file?: { path: string; contentDisposition: string; contentType: string };
    metadata?: unknown;
  }>;
  completed: boolean;
};

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

      // upload files to presign urls (sessionId required for S3 listener to update DynamoDB)
      await Promise.all(
        args.files.map((file, index) => {
          if (!result.files[index]?.url) {
            throw new Error(`No presign URL returned for file index ${index}`);
          }

          return fetch(result.files[index].url, {
            method: "PUT",
            headers: {
              "Content-Type": file.type,
              "x-amz-meta-x-ocel-session-id": result.sessionId,
            },
            body: file,
          });
        })
      );

      // poll for completion (S3 listener Lambda updates DynamoDB when upload lands)
      const pollParams = new URLSearchParams();
      pollParams.append("action", "poll");
      pollParams.append("uploader", String(uploader));
      pollParams.append("sessionId", result.sessionId);

      const pollUntilComplete = async (): Promise<PollResponse> => {
        const res = await fetch(`${url}?${pollParams.toString()}`, {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({}),
        });
        const data = (await res.json()) as PollResponse;
        if (data.completed) return data;
        await new Promise((r) => setTimeout(r, 1000));
        return pollUntilComplete();
      };

      const pollResult = await pollUntilComplete();

      if (args.onClientUploadComplete && pollResult.files) {
        const completedFiles = pollResult.files.filter(
          (f) => f.status === "SUCCESS" && f.file
        );
        const results = completedFiles.map((f) => ({
          file: f.file!,
          metadata: f.metadata ?? {},
        }));
        if (results.length > 0) {
          args.onClientUploadComplete(
            results.length === 1 ? results[0] : (results as any)
          );
        }
      }
    },
  };
}
