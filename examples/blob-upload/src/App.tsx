import { useState } from "react";
import { createUploadClient } from "ocel/blob/client";
import { storageBucket } from "@examples/hono/storage";

const apiUrl = import.meta.env.VITE_API_URL || "http://localhost:8001";
const client = createUploadClient<typeof storageBucket>({
  url: `${apiUrl}/upload`,
});

export default function App() {
  const [status, setStatus] = useState<string>("");
  const [files, setFiles] = useState<File[]>([]);
  const [result, setResult] = useState<string>("");

  const handleUpload = async () => {
    if (files.length === 0) {
      setStatus("Select files first");
      return;
    }

    setStatus("Presigning...");
    setResult("");

    try {
      await client.upload("avatars", {
        files,
        onClientUploadComplete: (res: unknown) => {
          setResult(JSON.stringify(res, null, 2));
        },
      });
      setStatus("Upload complete");
    } catch (err) {
      setStatus(`Error: ${err instanceof Error ? err.message : String(err)}`);
    }
  };

  return (
    <div>
      <h1>Ocel Blob Upload Test</h1>
      <p>Backend: {apiUrl}</p>

      <div style={{ marginBottom: "1rem" }}>
        <input
          type="file"
          multiple
          accept="image/*"
          onChange={(e) => setFiles(Array.from(e.target.files || []))}
        />
      </div>

      <button onClick={handleUpload} disabled={files.length === 0}>
        Upload
      </button>

      {status && (
        <p style={{ marginTop: "1rem", color: "#666" }}>{status}</p>
      )}

      {result && (
        <pre
          style={{
            marginTop: "1rem",
            padding: "1rem",
            background: "#f5f5f5",
            borderRadius: "4px",
            fontSize: "12px",
            overflow: "auto",
          }}
        >
          {result}
        </pre>
      )}
    </div>
  );
}
