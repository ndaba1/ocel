import type { IncomingMessage } from "node:http";
import z, { type ZodType } from "zod";
import type { KnownMimeType } from "./mime-types";
import type { UploadError } from "./upload-error";

type SuggestedSize = "1KB" | "100KB" | "1MB" | "10MB" | "100MB" | "1GB";
export type FileSizeUnit = "B" | "KB" | "MB" | "GB";
export type UnitAutocomplete<TUnit extends string> = `${number}${TUnit}`;
export type FileSize =
  | SuggestedSize
  | UnitAutocomplete<FileSizeUnit>
  | number
  | (string & {});

export type StringWithAutocomplete<T extends string> = T | (string & {});
export type HandleUploadBody = PresignUploadBody | CallbackBody<any>;
export type HandleUploadResult = PresignUploadResponse;

export type PresignUploadBody = {
  files: {
    name: string;
    size: number;
    mimeType: string;
    lastModified: number;
  }[];
  input?: Record<string, any>;
};

export type PresignUploadResponse = {
  files: {
    url: string;
    key: string;
    name: string;
  }[];
};

export type CallbackBody<TMeta> = {
  metadata: TMeta;
  file: {
    path: string;
    contentDisposition: string;
    contentType: string;
  };
};

export type UploadFileBody = {
  key: string;
  name: string;
  size: number;
  mimeType: string;
  disposition: string;
  lastModified: number;
  metadata?: Record<string, string>;
};

export type PathConfig = {
  prefix?: string;
  suffix?: string;
  randomSuffix?: boolean;
};

export type PathContext<TMeta> = [TMeta] extends [never]
  ? {
      /**
       * File extension including the dot (e.g., .jpg, .png)
       */
      ext: string;
      /**
       * File name without extension
       */
      fileNameWithoutExt: string;
      /**
       * Original file name as uploaded by user
       */
      fileNameWithExt: string;
    }
  : {
      /**
       * File extension including the dot (e.g., .jpg, .png)
       */
      ext: string;
      /**
       * File name without extension
       */
      fileNameWithoutExt: string;
      /**
       * Original file name as uploaded by user
       */
      fileNameWithExt: string;
      /**
       * Any metadata returned by middleware function
       */
      metadata: TMeta;
    };

export type PathGenerator<TMeta> = (ctx: PathContext<TMeta>) => string;
export type PathInput<TMeta> = PathConfig | PathGenerator<TMeta>;

export type TUploaderConfig<
  TInput extends ZodType = never,
  TMeta = never,
  TReturn = unknown
> = {
  path?: PathInput<TMeta>;
  input?: TInput;
  accept?: StringWithAutocomplete<KnownMimeType>[];
  limits?: {
    maxFileSize?: FileSize | ((ctx: { metadata?: TMeta }) => FileSize);
    maxFileCount?: number | ((ctx: { metadata?: TMeta }) => number);
    minFileCount?: number | ((ctx: { metadata?: TMeta }) => number);
  };
  contentDisposition?: "inline" | "attachment";

  onBeforeUpload: (
    opts: [TInput] extends [never]
      ? {
          files: PresignUploadBody["files"];
          req: Request | IncomingMessage;
        }
      : {
          files: PresignUploadBody["files"];
          input: z.infer<TInput>;
          req: Request | IncomingMessage;
        }
  ) => TMeta | Promise<TMeta>;
  onUploadError?: (args: { error: UploadError }) => void | Promise<void>;
  onUploadComplete?: (args: {
    metadata: TMeta;
    file: { path: string; contentDisposition: string; contentType: string };
  }) => TReturn | Promise<TReturn>;

  _def: {
    meta: TMeta;
    input: TInput;
    output: TReturn;
  };
};

export type TBucketConfig<TUploaderShape> = {
  uploaders: TUploaderShape;
};
