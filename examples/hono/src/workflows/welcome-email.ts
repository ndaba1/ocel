import { workflow } from "ocel/workflow";

export const sendEmailWorkflow = workflow("sendWelcomeEmail", {
  queue: { concurrencyLimit: 5 },
  async run({ ctx }) {
    console.log("Build Version: 2.0.0 - Testing OTEL Fix");

    await ctx.step("checkStatus", async () => {
      console.log("Checking service status...");
    });

    await ctx.step("makeApiCall", async (s) => {
      const data = await fetch("https://api.vercel.com/v1/health").then((r) =>
        r.json(),
      );

      s.logger.info("Email api response:", data);
      console.log("Email response:", data);
    });

    await ctx.wait("long-delay", { seconds: 90 });

    await ctx.step("finalize", async (s) => {
      // Finalization logic
      console.log("This code will log after a 90 seconds delay....");
    });
  },
});
export default sendEmailWorkflow;
