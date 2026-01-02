import { Entity } from "electrodb";
import { DynamoDBClient } from "@aws-sdk/client-dynamodb";

const client = new DynamoDBClient({});

export const UploadSession = new Entity(
  {
    model: {
      entity: "upload-session",
      version: "1",
      service: "blob",
    },
    attributes: {
      sessionId: {
        type: "string",
        required: true,
      },
      bucketName: {
        type: "string",
        required: true,
      },
      fileKey: {
        type: "string",
        required: true,
      },
      contentType: {
        type: "string",
        required: true,
      },
      contentDisposition: {
        type: "string",
        required: false,
      },
      fileSize: {
        type: "number",
        required: true,
      },
      createdAt: {
        type: "string",
        required: true,
        default: () => new Date().toISOString(),
      },
      updatedAt: {
        type: "string",
        required: true,
        default: () => new Date().toISOString(),
        set: () => new Date().toISOString(),
      },
      status: {
        type: ["PENDING", "SUCCESS", "FAILED"] as const,
        required: true,
        default: "PENDING",
      },
      metadata: {
        type: "string",
        required: false,
      },
    },
    indexes: {
      sessions: {
        pk: {
          field: "pk",
          composite: ["sessionId"],
          template: "SESSION#${sessionId}",
        },
        sk: {
          field: "sk",
          composite: ["bucketName", "fileKey"],
          template: "FILE#${bucketName}#${fileKey}",
        },
      },
    },
  },
  {
    client,
    table: "OcelTable", // required for all ocel logic
  }
);
