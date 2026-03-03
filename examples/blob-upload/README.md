# Blob Upload Test

Minimal Vite + React app to test the ocel blob upload flow end-to-end.

## Run

1. **Start the Hono backend** (in another terminal):

   ```bash
   cd examples/hono && bun run dev
   ```

   Backend runs at http://localhost:8001.

2. **Start the Vite app**:

   ```bash
   cd examples/blob-upload && bun run dev
   ```

   App runs at http://localhost:5173.

3. Select an image, click Upload, and verify the flow (presign → S3 upload → poll → callbacks).

## Env

- `VITE_API_URL` – Backend URL (default: http://localhost:8001)
