# Getting Started with Ocel

Ocel brings Vercel-like developer experience to AWS. Deploy serverless functions, blob storage, and workflows while keeping full ownership of your infrastructure and the raw power of AWS.

## Prerequisites

- **Node.js** 20+
- **Bun** (for builds)
- **AWS CLI** configured with credentials
- **OpenTofu** or Terraform (Ocel uses OpenTofu by default)

## Install

Add Ocel to your project:

```bash
pnpm add ocel
# or: npm install ocel
# or: yarn add ocel
```

## Bootstrap (one-time)

Before your first deploy, run bootstrap to create core AWS resources (DynamoDB table, assets bucket, etc.):

```bash
ocel bootstrap
```

This deploys a CloudFormation stack. You only need to run it once per AWS account/region.

## Create a project

Initialize an Ocel project:

```bash
ocel init
```

Walk through the prompts to set your project name, infra location, and environment. This creates `ocel.json` and the `.ocel` directory.

## Minimal app: Hono + Lambda + Blob

Create a minimal app with a Lambda and blob uploads. See `examples/hono` for the full code.

**ocel.json:**

```json
{
  "name": "my-app",
  "language": "typescript",
  "infraSources": ["./src/infra/**/*.ts"]
}
```

**src/infra/storage.ts** – define a bucket with uploaders:

```ts
import { bucket, uploader } from "ocel/blob";

const uploaders = {
  avatars: uploader(
    { middleware: async () => {} },
    { accept: ["image/*"], path: { prefix: "avatars/", randomSuffix: true } }
  ),
};

export const storageBucket = bucket("storageBucket", { uploaders });
```

**src/infra/index.ts** – Lambda with blob route, linked to the bucket:

```ts
import { lambda } from "ocel/lambda/hono";
import { Hono } from "hono";
import { storageBucket } from "./storage";
import { createRouteHandler } from "ocel/blob/hono";

const app = new Hono();
app.post("/upload", createRouteHandler(storageBucket));

export default lambda("honoApp", app, { link: [storageBucket] });
```

## Run locally

Start the dev server with hot reload:

```bash
ocel dev
```

Your Lambda and blob routes are available locally. Uploads go to S3 in your AWS account.

## Deploy

Deploy your infrastructure and app:

```bash
ocel deploy
```

Or run OpenTofu directly from the env directory (e.g. `.ocel/tofu/dev`).

## Roadmap

- **Alpha**: Lambda, blob (UploadThing-style), workflows. Current focus.
- **Beta**: More triggers, polish, improved DX.
- **GA**: Production-ready with full documentation and support.

## Troubleshooting

| Issue | Solution |
|-------|----------|
| "ocel package not found" when building | Run `pnpm add ocel` (or equivalent) in your project. |
| Bootstrap not run | Run `ocel bootstrap` before first deploy. |
| Provider init fails | Ensure AWS credentials are configured (`aws sts get-caller-identity`). |
| Tofu/apply errors | Check `.ocel/tofu/<env>/tofu_error.log` for details. |
