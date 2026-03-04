import { serve } from "@hono/node-server";
import { app } from "./infra";

serve({
  fetch: app.fetch,
  port: 8001,
}).on("listening", (p) => {
  console.log("Hono example app is running at http://localhost:8001");
});
