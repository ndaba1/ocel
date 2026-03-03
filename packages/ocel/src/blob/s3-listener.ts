/**
 * S3 event listener Lambda for ocel/blob.
 * Receives ObjectCreated events, reads x-ocel-session-id from object metadata,
 * and updates the UploadSession in DynamoDB to SUCCESS.
 */
import type { S3Handler, S3Event } from "aws-lambda";
import { HeadObjectCommand, S3Client } from "@aws-sdk/client-s3";
import {
  UpdateItemCommand,
  DynamoDBClient,
} from "@aws-sdk/client-dynamodb";

const s3 = new S3Client({});
const dynamo = new DynamoDBClient({});
const TABLE_NAME = process.env.OCEL_TABLE_NAME || "OcelTable";

export const handler: S3Handler = async (event: S3Event) => {
  for (const record of event.Records) {
    const bucket = record.s3.bucket.name;
    const key = decodeURIComponent(record.s3.object.key.replace(/\+/g, " "));

    try {
      const head = await s3.send(
        new HeadObjectCommand({ Bucket: bucket, Key: key })
      );
      const sessionId = head.Metadata?.["x-ocel-session-id"];
      if (!sessionId) continue;

      const pk = `SESSION#${sessionId}`;
      const sk = `FILE#${bucket}#${key}`;

      await dynamo.send(
        new UpdateItemCommand({
          TableName: TABLE_NAME,
          Key: {
            pk: { S: pk },
            sk: { S: sk },
          },
          UpdateExpression: "SET #status = :status, updatedAt = :now",
          ExpressionAttributeNames: { "#status": "status" },
          ExpressionAttributeValues: {
            ":status": { S: "SUCCESS" },
            ":now": { S: new Date().toISOString() },
          },
        })
      );
    } catch (err) {
      console.error(`Failed to update session for ${bucket}/${key}:`, err);
    }
  }
};
