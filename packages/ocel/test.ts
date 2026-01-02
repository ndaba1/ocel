import { XRayClient, BatchGetTracesCommand } from "@aws-sdk/client-xray";
import { writeFileSync } from "node:fs";

const traceIds = [
  //   "Root=1-69535932-0d90f9657324b5c051517de8;Parent=60d236ebeaea520d;Sampled=0;Lineage=1:bf22f9e3:0",
  // "1-695408c5-41e04a291c6ca19e7a1e3b6c",
  "1-69555768-6718c51a29aa4fff7ae28d1b",
];

const client = new XRayClient({});
// const client = new LambdaClient({});

const command = new BatchGetTracesCommand({
  TraceIds: traceIds,
});

const response = await client.send(command);

console.log("Traces:", JSON.stringify(response, null, 2));
const document = response.Traces?.[0]?.Segments?.[1]?.Document;
const target = response.Traces?.[0]?.Segments?.find((s) => {
  const doc = JSON.parse(s.Document || "{}");
  return doc.subsegments?.length > 0 && doc.aws?.["span.kind"] === "LOCAL_ROOT";
});
if (target) {
  writeFileSync(
    "./xray-trace-segment.json",
    JSON.stringify(JSON.parse(target.Document || "{}"), null, 2)
  );
}
writeFileSync("./xray-traces-response.json", JSON.stringify(response, null, 2));

// const command = new ListDurableExecutionsByFunctionCommand({
//   FunctionName: "sendWelcomeEmail-ocel-example-vndaba",
// });
// const response = await client.send(command);

// console.log("Durable Executions:", JSON.stringify(response, null, 2));

// const cmd2 = new GetDurableExecutionCommand({
//   DurableExecutionArn:
//     "arn:aws:lambda:us-east-1:150070262128:function:sendWelcomeEmail-ocel-example-vndaba:$LATEST/durable-execution/e2359f5e-ea4f-4b9c-b87e-5927203a5e04/926d6c2b-a6ce-39a6-a34f-17af048affeb",
// });

// const resp2 = await client.send(cmd2);

// console.log("Durable Execution Details:", JSON.stringify(resp2, null, 2));

// const cmd3 = new GetDurableExecutionHistoryCommand({
//   DurableExecutionArn:
//     "arn:aws:lambda:us-east-1:150070262128:function:sendWelcomeEmail-ocel-example-vndaba:$LATEST/durable-execution/e2359f5e-ea4f-4b9c-b87e-5927203a5e04/926d6c2b-a6ce-39a6-a34f-17af048affeb",
// });

// const resp3 = await client.send(cmd3);

// console.log("Durable Execution History:", JSON.stringify(resp3, null, 2));
