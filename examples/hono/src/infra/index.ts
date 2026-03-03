import { lambda } from "ocel/lambda/hono";
import { Hono } from "hono";
import { cors } from "hono/cors";
import { storageBucket } from "./storage";
import { createRouteHandler } from "ocel/blob/hono";

export const app = new Hono();

app.use("*", cors());

app.get("/", (c, _) => c.json({ status: "Hello World, from Hono !!!!" }));
app.post("/upload", createRouteHandler(storageBucket));

export default lambda("honoApp", app, { link: [storageBucket] });
