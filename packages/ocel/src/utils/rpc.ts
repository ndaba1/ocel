declare global {
  var __ocelRegister: Promise<unknown>[];
}

export const rpc = {
  register: async (args: {
    id: string;
    type: "lambda" | "bucket" | "workflow" | "postgres";
    source: string;
    config?: Record<string, unknown>;
  }) => {
    const SERVER_URL = process.env.OCEL_SERVER;
    if (!SERVER_URL) {
      throw new Error("OCEL_SERVER environment variable is not set");
    }

    const p = fetch(`${SERVER_URL}/register`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
      },
      body: JSON.stringify(args),
    });

    globalThis.__ocelRegister ??= [];
    globalThis.__ocelRegister.push(p);
  },
};
