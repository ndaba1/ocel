import type { IncomingMessage } from "node:http";
import type { FileSize } from "./types";
import { UploadError } from "./upload-error";

export function parseReq(req: Request | IncomingMessage): {
  headers: Record<string, string>;
  method: string;
  url: string;
  path: string;
  query: URLSearchParams;
} {
  if (req instanceof Request) {
    const headers: Record<string, string> = {};
    req.headers.forEach((value, key) => {
      headers[key] = value;
    });
    return {
      headers,
      method: req.method,
      url: req.url,
      path: new URL(req.url).pathname,
      query: new URL(req.url).searchParams,
    };
  }

  const headers: Record<string, string> = {};
  for (const [key, value] of Object.entries(req.headers)) {
    if (typeof value === "string") {
      headers[key] = value;
    } else if (Array.isArray(value)) {
      headers[key] = value.join(", ");
    }
  }

  const url = new URL(req.url || "", "http://localhost");

  return {
    headers,
    method: req.method || "GET",
    url: req.url || "",
    path: url.pathname,
    query: url.searchParams,
  };
}

/**
 * Converts a human-readable file size string (e.g., "10MB", "2.5GB") into bytes.
 * Throws an error for invalid input formats.
 * * @param size The file size represented as a number (bytes) or a string with units (KB, MB, GB, B).
 * @returns The size in bytes (number).
 */
export function parseFileSize(size: FileSize): number {
  if (typeof size === "number") {
    if (size < 0) {
      throw UploadError.badRequest("File size cannot be negative.");
    }
    return size;
  }

  // Convert the input to uppercase to handle 'mb', 'MB', 'Mb' uniformly
  const cleanSize = size.toUpperCase().trim();

  // Regular expression to extract the numeric value and the unit
  // Captures: (1) The number part, (2) The unit part (B, KB, MB, GB)
  const match = cleanSize.match(/^(\d+(\.\d+)?)([A-Z]+)$/);

  if (!match) {
    throw UploadError.badRequest(
      `Invalid file size format: "${size}". Expected formats: "10MB", "2GB", "512KB", or raw bytes.`
    );
  }

  const numericValue = Number.parseFloat(match[1] ?? "0");
  const unit = match[3] ?? "";

  const byteConversionMap: Record<string, number> = {
    B: 1,
    KB: 1024,
    MB: 1024 ** 2, // 1,048,576
    GB: 1024 ** 3, // 1,073,741,824
  };

  const multiplier = byteConversionMap[unit];

  if (!multiplier) {
    throw new Error(
      `Invalid file size unit: "${unit}". Must be B, KB, MB, or GB.`
    );
  }

  // Return the size rounded to the nearest integer byte
  return Math.round(numericValue * multiplier);
}
