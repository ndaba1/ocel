import slugify from "@sindresorhus/slugify";
import type {
  CallbackBody,
  HandleUploadBody,
  PathConfig,
  PathGenerator,
  PresignUploadBody,
  TUploaderConfig,
  UploadFileBody,
} from "./types";
import { UploadError } from "./upload-error";
import type { IncomingMessage } from "node:http";
import path from "node:path";
import { PutObjectCommand, S3Client } from "@aws-sdk/client-s3";
import { getNanoid } from "../utils/nanoid";
import { UploadSession } from "../internal/db";
import { getSignedUrl } from "@aws-sdk/s3-request-presigner";
import { IS_DEV_MODE } from "../utils/constants";
import type { ZodType } from "zod";
import { parseFileSize, parseReq } from "./parse";
import type { Bucket } from "./bucket";

export class Uploader<TMeta = unknown> {
  private pathGenerator: PathGenerator<TMeta> | null = null;

  constructor(
    private bucket: Bucket<any>,
    private name: string,
    private config: TUploaderConfig<any, TMeta>
  ) {
    const path = this.config.path;
    if (path) {
      if (typeof path === "function") {
        this.pathGenerator = path;
      } else {
        this.pathGenerator = this.createConfigBasedGenerator(path);
      }
    }
  }

  async handleUpload({
    action,
    body,
    request,
  }: {
    request: Request | IncomingMessage;
    body: HandleUploadBody;
    action: "presign" | "callback" | "poll";
  }) {
    try {
      const req = parseReq(request);

      switch (action) {
        case "presign": {
          if (!("files" in body)) {
            throw UploadError.badRequest("Invalid presign body");
          }

          const presignBody = body as PresignUploadBody;
          const rawInput = presignBody.input || {};
          const parser = this.config.input as ZodType | undefined;

          let parsedInput: any;

          // TODO: error handling
          if (parser) {
            const parsed = parser.safeParse(rawInput);
            if (!parsed.success) {
              throw UploadError.badRequest(parsed.error.issues[0]?.message);
            }

            parsedInput = parsed.data;
          }

          const middleware = this.config.onBeforeUpload;
          const customMeta = await middleware?.({
            req: request,
            input: parsedInput,
            files: presignBody.files,
          });

          const metadata = {
            "x-ocel-path": req.path,
            "x-ocel-uploader": this.name,
            ...(customMeta
              ? { "x-ocel-metadata": JSON.stringify(customMeta) }
              : {}),
          };

          // TODO: validate files against uploader config (size, count, types, etc)
          this.validateFiles(presignBody.files, customMeta);

          const files = body.files.map((f) => {
            const name = slugify(f.name, { preserveCharacters: ["."] });

            return {
              ...f,
              key:
                this.pathGenerator?.({
                  fileNameWithExt: name,
                  metadata: customMeta as any,
                  fileNameWithoutExt: name.replace(path.extname(name), ""),
                  ext: path.extname(name),
                } as any) || name,
              disposition: "inline",
              metadata: {
                ...metadata,
                "x-ocel-original-filename": f.name,
              },
            };
          }) satisfies UploadFileBody[];

          const provider = this.getProvider();

          return provider.presign(files);
        }
        case "poll": {
          // TODO: response flushing for SSE

          return {};
        }
        case "callback": {
          // TODO: validate callback body
          if (!("metadata" in body)) {
            throw UploadError.badRequest("Invalid callback body");
          }

          const data = body as CallbackBody<TMeta>;

          await this.config.onUploadComplete?.(data);

          return {
            type: "UPLOAD_COMPLETED" as const,
            response: "ok" as const,
          };
        }
        default:
          throw UploadError.badRequest(`Unhandled action type: ${action}`);
      }
    } catch (err) {
      if (err instanceof UploadError) {
        this.config.onUploadError?.({ error: err });

        return {
          error: {
            message: err.message,
            code: err.code, // e.g. "FORBIDDEN"
            status: err.statusCode, // e.g. 403
          },
        };
      }

      return {
        error: {
          message: "Internal Server Error",
          code: "INTERNAL_SERVER_ERROR",
          status: 500,
        },
      };
    }
  }

  getProvider() {
    const bucket = this.bucket;
    const config = this.config;
    const bucketId = bucket.__name();

    if (IS_DEV_MODE) {
      return {
        presign: async (files: UploadFileBody[]) => {
          const s3Client = new S3Client();
          const sessionId = getNanoid(8);

          const presignedFiles = await Promise.all(
            files.map(async (file) => {
              // create new session and set status to pending
              await UploadSession.create({
                sessionId,
                bucketName: bucketId,
                fileKey: file.key,
                createdAt: new Date().toISOString(),
                status: "PENDING",
                contentType: file.mimeType,
                contentDisposition: file.disposition,
                fileSize: file.size,
                metadata: file.metadata
                  ? JSON.stringify(file.metadata)
                  : undefined,
              }).go();

              const url = await getSignedUrl(
                s3Client,
                new PutObjectCommand({
                  Key: file.key,
                  Bucket: bucketId,
                  // TODO: find out why this breaks uploads
                  // ContentDisposition: file.disposition,
                  // ContentLength: file.size,
                  ContentType: file.mimeType,
                  Metadata: file.metadata,
                }),
                { expiresIn: 3600 }
              );

              return {
                url,
                key: file.key,
                name: file.name,
              };
            })
          );

          const startPolling = async () => {
            const pendingFiles = new Set(files.map((f) => f.key));
            let attempts = 0;
            const MAX_ATTEMPTS = 120;

            const checkStatus = async () => {
              // Stop if nothing left to track or timeout reached
              if (pendingFiles.size === 0 || attempts >= MAX_ATTEMPTS) return;

              attempts++;

              try {
                // Query all files in this session
                const result = await UploadSession.query
                  .sessions({ sessionId })
                  .go();

                for (const record of result.data) {
                  // Only process files we are actively tracking
                  if (!pendingFiles.has(record.fileKey)) continue;

                  if (record.status === "SUCCESS") {
                    pendingFiles.delete(record.fileKey);

                    let parsedMeta: TMeta = {} as TMeta;
                    try {
                      if (record.metadata) {
                        parsedMeta = JSON.parse(record.metadata);
                      }
                    } catch (e) {
                      console.warn("Failed to parse metadata for callback", e);
                    }

                    await config.onUploadComplete?.({
                      file: {
                        path: record.fileKey,
                        contentDisposition:
                          record.contentDisposition || "inline",
                        contentType: record.contentType,
                      },
                      metadata: parsedMeta,
                    });
                  } else if (record.status === "FAILED") {
                    // Stop tracking failed files so we don't loop forever
                    pendingFiles.delete(record.fileKey);

                    config.onUploadError?.({
                      error: new UploadError(
                        `Upload failed for ${record.fileKey}`,
                        500,
                        "UPLOAD_FAILED"
                      ),
                    });
                  }
                }
              } catch (err) {
                console.error("Polling error (will retry):", err);
              }

              if (pendingFiles.size > 0) {
                setTimeout(checkStatus, 1000);
              }
            };

            checkStatus();
          };

          startPolling();

          return {
            files: presignedFiles,
          };
        },
      };
    }

    return {
      presign: async () => {},
    };
  }

  createConfigBasedGenerator(config: PathConfig): PathGenerator<any> {
    return (ctx) => {
      let key = ctx.fileNameWithoutExt;

      // random suffix
      if (config.randomSuffix) {
        key += `-${Math.random().toString(36).slice(2, 10)}`;
      }

      // static suffix
      else if (config.suffix) {
        key += `-${config.suffix}`;
      }

      // add extension
      key += ctx.ext;

      // prefix
      if (config.prefix) {
        return `${config.prefix.replace(/\/$/, "")}/${key}`;
      }

      return key;
    };
  }

  private validateFiles(
    files: PresignUploadBody["files"],
    metadata?: TMeta
  ): void {
    const { accept, limits } = this.config;

    if (limits?.maxFileCount !== undefined) {
      const max =
        typeof limits.maxFileCount === "function"
          ? limits.maxFileCount({ metadata })
          : limits.maxFileCount;

      if (files.length > max) {
        throw UploadError.badRequest(
          `File limit exceeded. Maximum files allowed: ${max}.`
        );
      }
    }
    if (limits?.minFileCount !== undefined) {
      const min =
        typeof limits.minFileCount === "function"
          ? limits.minFileCount({ metadata })
          : limits.minFileCount;

      if (files.length < min) {
        throw UploadError.badRequest(
          `File limit not met. Minimum files required: ${min}.`
        );
      }
    }

    // 2. Per-File Validation (Size and MIME Type)
    for (const file of files) {
      // MIME Type Validation
      if (accept && accept.length > 0) {
        const isAccepted = accept.some((allowedType) => {
          // Simple check for wildcards (*/*) and direct matches (image/png)
          if (allowedType.endsWith("/*")) {
            const typeGroup = allowedType.slice(0, -2);
            return file.mimeType.startsWith(typeGroup);
          }
          return file.mimeType === allowedType;
        });

        if (!isAccepted) {
          throw UploadError.badRequest(
            `File type not allowed: ${
              file.mimeType
            }. Must be one of: ${accept.join(", ")}.`
          );
        }
      }

      // Max File Size Validation
      if (limits?.maxFileSize !== undefined) {
        const sizeValue =
          typeof limits.maxFileSize === "function"
            ? limits.maxFileSize({ metadata })
            : limits.maxFileSize;

        const maxSizeInBytes = parseFileSize(sizeValue);

        if (file.size > maxSizeInBytes) {
          // You might want to convert maxSize from bytes to KB/MB for a better error message
          throw UploadError.badRequest(
            `File size too large. Maximum size: ${maxSizeInBytes} bytes.`
          );
        }
      }
    }
  }
}
